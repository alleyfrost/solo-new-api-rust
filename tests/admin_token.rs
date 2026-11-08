use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Request, Uri};
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::auth::admin::AdminContext;
use solo_new_api_rust::state::{AppState, Caches};

async fn setup() -> (Router, SqlitePool, AppState) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    // 生产库带 idx_tokens_key 唯一索引且不区分 deleted_at，重键（含与软删行重键）在库层就会被拒
    sqlx::query("CREATE UNIQUE INDEX idx_tokens_key ON tokens(`key`)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE quota_data (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,username TEXT DEFAULT '',model_name TEXT DEFAULT '',created_at INTEGER,use_group TEXT DEFAULT '',token_id INTEGER DEFAULT 0,channel_id INTEGER DEFAULT 0,node_name TEXT DEFAULT '',token_used INTEGER DEFAULT 0,count INTEGER DEFAULT 0,quota INTEGER DEFAULT 0,cache_read_tokens INTEGER DEFAULT 0,cache_creation_tokens INTEGER DEFAULT 0)").execute(&pool).await.unwrap();
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
    let app = solo_new_api_rust::routes::admin::token::router()
        .layer(axum::Extension(AdminContext {
            user_id: 1,
            sid: "test".into(),
        }))
        .with_state(state.clone());
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

    sqlx::query("INSERT INTO quota_data (user_id,model_name,created_at,use_group,token_id,channel_id,token_used,count,cache_read_tokens) VALUES (1,'m1',100,'default',?,3,120,4,30)")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, list) = call(&app, "GET", "/api/token/?p=1&size=10", None).await;
    assert_eq!(list["data"]["total"], 1);
    assert_eq!(list["data"]["items"][0]["name"], "cli");
    assert_eq!(list["data"]["items"][0]["key"], masked);
    assert_eq!(list["data"]["items"][0]["request_count"], 4);
    assert_eq!(list["data"]["items"][0]["used_tokens"], 120);
    assert_eq!(list["data"]["items"][0]["cache_read_tokens"], 30);

    let (_, full) = call(&app, "POST", &format!("/api/token/{id}/key"), None).await;
    assert_eq!(full["data"]["key"], key);
    let (_, found) = call(
        &app,
        "GET",
        "/api/token/search?keyword=cli&p=1&size=10",
        None,
    )
    .await;
    assert_eq!(found["data"]["total"], 1);
    let (_, none) = call(
        &app,
        "GET",
        "/api/token/search?keyword=zzz&p=1&size=10",
        None,
    )
    .await;
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

/// 16–64 位、只含 `[A-Za-z0-9_]` 的自填密钥样例
const CUSTOM_KEY: &str = "my_custom_key_2026";

#[tokio::test]
async fn create_accepts_custom_key_and_strips_sk_prefix() {
    let (app, pool, state) = setup().await;

    let (status, body) = call(
        &app,
        "POST",
        "/api/token/",
        Some(serde_json::json!({"name": "cli", "key": format!("sk-{CUSTOM_KEY}")})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id = body["data"]["id"].as_i64().unwrap();
    assert_eq!(body["data"]["key"], "my_c**********2026");

    let stored: String = sqlx::query_scalar("SELECT `key` FROM tokens WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        stored, CUSTOM_KEY,
        "入库必须去掉 sk- 前缀，与随机生成的密钥同构"
    );

    // 客户端仍按完整 sk-... 使用：认证侧剥一层前缀后命中缓存
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer sk-{CUSTOM_KEY}")).unwrap(),
    );
    let uri = Uri::from_static("/v1/chat/completions");
    let extracted =
        solo_new_api_rust::auth::token::extract_key(&headers, &uri).expect("authorization header");
    assert_eq!(extracted, CUSTOM_KEY);
    let ctx = solo_new_api_rust::auth::token::validate_token(&state, &extracted).unwrap();
    assert_eq!(ctx.token_id, id);
    assert_eq!(ctx.token_name, "cli");

    // 不带前缀直接填裸密钥同样可用
    let (_, second) = call(
        &app,
        "POST",
        "/api/token/",
        Some(serde_json::json!({"name": "sdk", "key": "bare_key_abcdefghij"})),
    )
    .await;
    let second_id = second["data"]["id"].as_i64().unwrap();
    let second_stored: String = sqlx::query_scalar("SELECT `key` FROM tokens WHERE id=?")
        .bind(second_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(second_stored, "bare_key_abcdefghij");
}

#[tokio::test]
async fn blank_key_falls_back_to_generated_key() {
    let (app, pool, _) = setup().await;

    // 前端留空时提交空串，而不是省略字段
    for raw in ["", "   ", "sk-"] {
        let (status, body) = call(
            &app,
            "POST",
            "/api/token/",
            Some(serde_json::json!({"name": "auto", "key": raw})),
        )
        .await;
        assert_eq!(status, 200, "{body} for {raw:?}");
        let id = body["data"]["id"].as_i64().unwrap();
        let stored: String = sqlx::query_scalar("SELECT `key` FROM tokens WHERE id=?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored.len(), 48, "留空回落随机 48 位密钥：{stored}");
    }
}

#[tokio::test]
async fn create_rejects_invalid_custom_keys() {
    let (app, pool, _) = setup().await;
    let long_key = "a".repeat(65);
    let cases = [
        ("abc", "Token key must be 16-64 characters"),
        (long_key.as_str(), "Token key must be 16-64 characters"),
        (
            "my-key-with-hyphens",
            "Token key may only contain letters, digits or underscore",
        ),
        (
            "密钥_not_ascii_key",
            "Token key may only contain letters, digits or underscore",
        ),
    ];

    for (raw, expected) in cases {
        let (status, body) = call(
            &app,
            "POST",
            "/api/token/",
            Some(serde_json::json!({"name": "cli", "key": raw})),
        )
        .await;
        assert_eq!(status, 400, "{body} for {raw:?}");
        assert_eq!(body["success"], false, "{body}");
        assert_eq!(body["message"], expected, "for {raw:?}");
        assert_eq!(
            body["message_key"], expected,
            "前端只翻译 message_key：for {raw:?}"
        );
    }

    // 其余管理端错误暂未补 message_key，保持纯英文 message。
    let too_long_name = "x".repeat(51);
    let (status, body) = call(
        &app,
        "POST",
        "/api/token/",
        Some(serde_json::json!({"name": too_long_name})),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body.get("message_key").is_none(),
        "未标注的错误不应带 message_key：{body}"
    );

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "非法密钥不得留下半成品行");
}

#[tokio::test]
async fn create_rejects_duplicate_key_including_soft_deleted_rows() {
    let (app, pool, _) = setup().await;
    let (_, created) = call(
        &app,
        "POST",
        "/api/token/",
        Some(serde_json::json!({"name": "first", "key": CUSTOM_KEY})),
    )
    .await;
    let id = created["data"]["id"].as_i64().unwrap();

    let (status, body) = call(
        &app,
        "POST",
        "/api/token/",
        Some(serde_json::json!({"name": "second", "key": CUSTOM_KEY})),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["message"], "Token key is already in use");
    assert_eq!(body["message_key"], "Token key is already in use");

    // 软删行仍在库里，idx_tokens_key 不看 deleted_at，所以重键依旧被拒
    let (status, _) = call(&app, "DELETE", &format!("/api/token/{id}/"), None).await;
    assert_eq!(status, 200);
    let (status, body) = call(
        &app,
        "POST",
        "/api/token/",
        Some(serde_json::json!({"name": "third", "key": CUSTOM_KEY})),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["message"], "Token key is already in use");
    assert_eq!(body["message_key"], "Token key is already in use");

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn update_ignores_key_so_existing_clients_keep_working() {
    let (app, pool, state) = setup().await;
    let (_, created) = call(
        &app,
        "POST",
        "/api/token/",
        Some(serde_json::json!({"name": "cli", "key": CUSTOM_KEY})),
    )
    .await;
    let id = created["data"]["id"].as_i64().unwrap();

    let (status, body) = call(
        &app,
        "PUT",
        "/api/token/",
        Some(serde_json::json!({
            "id": id,
            "name": "renamed",
            "key": "sk-a_totally_new_key_99"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"]["name"], "renamed");
    assert_eq!(body["data"]["key"], "my_c**********2026");

    let stored: String = sqlx::query_scalar("SELECT `key` FROM tokens WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        stored, CUSTOM_KEY,
        "编辑不改密钥：已下发的客户端配置不能静默失效"
    );
    assert!(
        state
            .caches
            .read()
            .unwrap()
            .token_by_key(CUSTOM_KEY)
            .is_some()
    );
}
