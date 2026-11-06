use axum::body::Body;
use axum::http::Request;
use axum::Router;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::state::{AppState, Caches};

async fn setup() -> (Router, SqlitePool) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE models (id INTEGER PRIMARY KEY AUTOINCREMENT, model_name TEXT, description TEXT, icon TEXT, tags TEXT, vendor_id INTEGER, endpoints TEXT, status INTEGER, sync_official INTEGER, created_time INTEGER, updated_time INTEGER, deleted_at TEXT, name_rule INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)").execute(&pool).await.unwrap();

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
    let app = solo_new_api_rust::routes::admin::models::router().with_state(state);
    (app, pool)
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
async fn model_crud_and_status_filter() {
    let (app, _pool) = setup().await;
    let (status, body) = call(
        &app,
        "POST",
        "/api/models/",
        Some(serde_json::json!({
            "model_name": "m1", "description": "demo", "status": 1
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id = body["data"]["id"].as_i64().unwrap();

    let (_, list) = call(&app, "GET", "/api/models/?p=1&size=10", None).await;
    assert_eq!(list["data"]["total"], 1);
    assert_eq!(list["data"]["items"][0]["model_name"], "m1");

    let (_, filtered) = call(&app, "GET", "/api/models/search?keyword=m2", None).await;
    assert_eq!(filtered["data"]["total"], 0);

    let (status, updated) = call(
        &app,
        "PUT",
        "/api/models/",
        Some(serde_json::json!({"id": id, "model_name": "m2", "description": "renamed"})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(updated["data"]["model_name"], "m2");

    let (_, _) = call(
        &app,
        "PUT",
        "/api/models/?status_only=true",
        Some(serde_json::json!({"id": id, "status": 0})),
    )
    .await;
    let enabled: Vec<String> = sqlx::query_scalar::<_, String>("SELECT model_name FROM models WHERE status=1")
        .fetch_all(&_pool)
        .await
        .unwrap();
    assert!(enabled.is_empty());

    let (status, _) = call(&app, "DELETE", &format!("/api/models/{id}"), None).await;
    assert_eq!(status, 200);
    let (_, list) = call(&app, "GET", "/api/models/?p=1&size=10", None).await;
    assert_eq!(list["data"]["total"], 0);
}
