use axum::body::Body;
use axum::http::Request;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::state::{AppState, Caches};

async fn schema() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE models (id INTEGER PRIMARY KEY AUTOINCREMENT, model_name TEXT, description TEXT, icon TEXT, tags TEXT, vendor_id INTEGER, endpoints TEXT, status INTEGER, sync_official INTEGER, created_time INTEGER, updated_time INTEGER, deleted_at TEXT, name_rule INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE vendors (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, description TEXT, icon TEXT, status INTEGER, created_time INTEGER, updated_time INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)").execute(&pool).await.unwrap();
    pool
}

async fn app(pool: &SqlitePool) -> Router {
    let caches = Caches::load(pool).await.unwrap();
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(cfg, pool.clone(), caches, b"test-secret".to_vec(), None);
    solo_new_api_rust::routes::admin::models::router().with_state(state)
}

async fn setup() -> (Router, SqlitePool) {
    let pool = schema().await;
    let app = app(&pool).await;
    (app, pool)
}

async fn insert_channel(pool: &SqlitePool, id: i64, name: &str, channel_type: i64, status: i64, models: &str) {
    sqlx::query("INSERT INTO channels (id, type, `key`, status, name, models, `group`) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(id)
        .bind(channel_type)
        .bind("sk-test")
        .bind(status)
        .bind(name)
        .bind(models)
        .bind("default")
        .execute(pool)
        .await
        .unwrap();
}

async fn insert_ability(pool: &SqlitePool, group: &str, model: &str, channel_id: i64, enabled: bool) {
    sqlx::query("INSERT INTO abilities (`group`, model, channel_id, enabled, priority, weight) VALUES (?, ?, ?, ?, 0, 0)")
        .bind(group)
        .bind(model)
        .bind(channel_id)
        .bind(enabled)
        .execute(pool)
        .await
        .unwrap();
}

/// 两个渠道：alpha 启用并挂了 a-model/b-model，beta 已禁用但配置了 c-model。
async fn setup_channels() -> (Router, SqlitePool) {
    let pool = schema().await;
    insert_channel(&pool, 1, "alpha", 1, 1, "a-model,b-model").await;
    insert_channel(&pool, 2, "beta", 14, 2, "c-model").await;
    insert_ability(&pool, "default", "a-model", 1, true).await;
    insert_ability(&pool, "vip", "a-model", 1, true).await;
    insert_ability(&pool, "default", "b-model", 1, true).await;
    insert_ability(&pool, "default", "c-model", 2, true).await;
    let app = app(&pool).await;
    (app, pool)
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
    let res = app.clone().oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn item_by_name<'a>(items: &'a [Value], name: &str) -> &'a Value {
    items
        .iter()
        .find(|item| item["model_name"] == name)
        .unwrap_or_else(|| panic!("no item {name} in {items:?}"))
}

#[tokio::test]
async fn model_crud_and_status_filter() {
    let (app, _pool) = setup().await;
    let (status, body) = call(
        &app,
        "POST",
        "/api/models/",
        Some(json!({"model_name": "m1", "description": "demo", "status": 1})),
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
        Some(json!({"id": id, "model_name": "m2", "description": "renamed"})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(updated["data"]["model_name"], "m2");

    call(&app, "PUT", "/api/models/?status_only=true", Some(json!({"id": id, "status": 0}))).await;
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

#[tokio::test]
async fn channel_models_are_unioned_with_states() {
    let (app, _pool) = setup_channels().await;

    // 不带 include_channel_models 时只看元数据，所以是空列表。
    let (_, plain) = call(&app, "GET", "/api/models/?p=1&page_size=10", None).await;
    assert_eq!(plain["data"]["total"], 0);

    let (status, list) = call(
        &app,
        "GET",
        "/api/models/?include_channel_models=true&p=1&page_size=10",
        None,
    )
    .await;
    assert_eq!(status, 200, "{list}");
    let data = &list["data"];
    assert_eq!(data["total"], 3);
    let items = data["items"].as_array().unwrap();
    let names: Vec<&str> = items.iter().map(|item| item["model_name"].as_str().unwrap()).collect();
    assert_eq!(names, ["a-model", "b-model", "c-model"]);

    let a = item_by_name(items, "a-model");
    assert_eq!(a["id"], 0);
    assert_eq!(a["has_metadata"], false);
    assert_eq!(a["square_state"], "visible");
    assert_eq!(a["configured_channel_count"], 1);
    assert_eq!(a["bound_channels"], json!([{"name": "alpha", "type": 1}]));
    assert_eq!(a["enable_groups"], json!(["default", "vip"]));

    // 禁用渠道里的模型仍算“已配置”，但没有可用路由，所以不展示。
    let c = item_by_name(items, "c-model");
    assert_eq!(c["square_state"], "unavailable");
    assert_eq!(c["configured_channel_count"], 1);
    assert_eq!(c["bound_channels"], json!([]));
}

#[tokio::test]
async fn metadata_row_replaces_synthetic_and_can_hide_it() {
    let (app, _pool) = setup_channels().await;
    let (_, created) = call(
        &app,
        "POST",
        "/api/models/",
        Some(json!({"model_name": "a-model", "status": 0})),
    )
    .await;
    let id = created["data"]["id"].as_i64().unwrap();
    assert_eq!(created["data"]["square_state"], "hidden");

    let (_, list) = call(
        &app,
        "GET",
        "/api/models/?include_channel_models=true&p=1&page_size=10",
        None,
    )
    .await;
    let items = list["data"]["items"].as_array().unwrap();
    assert_eq!(list["data"]["total"], 3, "a-model 不应同时出现元数据行与合成行");
    let a = item_by_name(items, "a-model");
    assert_eq!(a["id"], id);
    assert_eq!(a["has_metadata"], true);
    assert_eq!(a["square_state"], "hidden");
    assert_eq!(item_by_name(items, "b-model")["square_state"], "visible");
}

#[tokio::test]
async fn rule_rows_aggregate_matched_models() {
    let (app, _pool) = setup_channels().await;
    // name_rule=2 是包含匹配，会命中 a-model、b-model、c-model。
    let (_, created) = call(
        &app,
        "POST",
        "/api/models/",
        Some(json!({"model_name": "-model", "name_rule": 2, "status": 1})),
    )
    .await;
    let id = created["data"]["id"].as_i64().unwrap();

    let (_, list) = call(
        &app,
        "GET",
        "/api/models/search?include_channel_models=true&keyword=model",
        None,
    )
    .await;
    let items = list["data"]["items"].as_array().unwrap();
    let rule = item_by_name(items, "-model");
    assert_eq!(rule["id"], id);
    // 配置计数看全部渠道（含禁用），匹配模型名只看可用路由，所以 c-model 不进 matched。
    assert_eq!(rule["configured_channel_count"], 2);
    assert_eq!(rule["matched_count"], 2);
    assert_eq!(rule["matched_models"], json!(["a-model", "b-model"]));
    assert_eq!(rule["enable_groups"], json!(["default", "vip"]));
    // 三个具体名里两个可见、一个不可见，聚合结果是部分展示。
    assert_eq!(rule["square_state"], "partial");
}

#[tokio::test]
async fn square_state_filter_pages_and_paginates() {
    let (app, _pool) = setup_channels().await;

    let (_, first) = call(
        &app,
        "GET",
        "/api/models/search?include_channel_models=true&square_state=visible&p=1&page_size=1",
        None,
    )
    .await;
    assert_eq!(first["data"]["total"], 2);
    let items = first["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["model_name"], "a-model");

    let (_, second) = call(
        &app,
        "GET",
        "/api/models/search?include_channel_models=true&square_state=visible&p=2&page_size=1",
        None,
    )
    .await;
    assert_eq!(
        second["data"]["items"].as_array().unwrap()[0]["model_name"],
        "b-model"
    );

    let (_, beyond) = call(
        &app,
        "GET",
        "/api/models/search?include_channel_models=true&square_state=visible&p=99&page_size=1",
        None,
    )
    .await;
    assert!(beyond["data"]["items"].as_array().unwrap().is_empty());

    let (_, unavailable) = call(
        &app,
        "GET",
        "/api/models/search?include_channel_models=true&square_state=unavailable",
        None,
    )
    .await;
    assert_eq!(unavailable["data"]["total"], 1);

    let (status, invalid) = call(&app, "GET", "/api/models/search?square_state=nonsense", None).await;
    assert_eq!(status, 400, "{invalid}");
}

#[tokio::test]
async fn metadata_only_filters_disable_the_channel_union() {
    let (app, _pool) = setup_channels().await;
    call(&app, "POST", "/api/models/", Some(json!({"model_name": "a-model", "status": 1, "sync_official": 0}))).await;

    // status/sync/vendor 只作用于元数据，此时不再拼接渠道派生名。
    let (_, by_status) = call(
        &app,
        "GET",
        "/api/models/search?include_channel_models=true&status=enabled",
        None,
    )
    .await;
    assert_eq!(by_status["data"]["total"], 1);

    let (_, by_sync) = call(
        &app,
        "GET",
        "/api/models/search?include_channel_models=true&sync_official=no",
        None,
    )
    .await;
    assert_eq!(by_sync["data"]["total"], 1);

    let (_, no_filter) = call(
        &app,
        "GET",
        "/api/models/search?include_channel_models=true&status=all",
        None,
    )
    .await;
    assert_eq!(no_filter["data"]["total"], 3);
}

#[tokio::test]
async fn vendor_counts_and_vendor_list() {
    let (app, pool) = setup().await;
    sqlx::query("INSERT INTO vendors (id, name, description, icon, status, created_time, updated_time) VALUES (7, 'Acme', '', '', 1, 1, 1)")
        .execute(&pool)
        .await
        .unwrap();
    call(&app, "POST", "/api/models/", Some(json!({"model_name": "m1", "vendor_id": 7}))).await;
    call(&app, "POST", "/api/models/", Some(json!({"model_name": "m2"}))).await;

    let (_, list) = call(&app, "GET", "/api/models/?p=1&page_size=10", None).await;
    assert_eq!(list["data"]["vendor_counts"]["7"], 1);
    assert_eq!(list["data"]["vendor_counts"]["0"], 1);

    let (_, vendors) = call(&app, "GET", "/api/vendors/?p=1&page_size=1000", None).await;
    let items = vendors["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Acme");

    let (_, by_vendor) = call(&app, "GET", "/api/models/search?vendor=7", None).await;
    assert_eq!(by_vendor["data"]["total"], 1);
    assert_eq!(by_vendor["data"]["items"][0]["model_name"], "m1");
}
