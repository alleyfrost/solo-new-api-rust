use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::auth::admin::AdminContext;
use solo_new_api_rust::state::{AppState, Caches};

async fn pool_with_tables() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE UNIQUE INDEX idx_tokens_key ON tokens(`key`)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE token_rate_limits (token_id INTEGER PRIMARY KEY, rpm INTEGER NOT NULL DEFAULT 0, concurrency INTEGER NOT NULL DEFAULT 0)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE quota_data (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,username TEXT DEFAULT '',model_name TEXT DEFAULT '',created_at INTEGER,use_group TEXT DEFAULT '',token_id INTEGER DEFAULT 0,channel_id INTEGER DEFAULT 0,node_name TEXT DEFAULT '',token_used INTEGER DEFAULT 0,count INTEGER DEFAULT 0,quota INTEGER DEFAULT 0,cache_read_tokens INTEGER DEFAULT 0,cache_creation_tokens INTEGER DEFAULT 0)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users VALUES (1,'root','x',NULL,100,1,'default',1,NULL,NULL)")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

async fn state_with_pool(pool: SqlitePool) -> AppState {
    let caches = Caches::load(&pool).await.unwrap();
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    AppState::new(cfg, pool, caches, b"test-secret".to_vec(), None)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    headers: Option<HeaderMap>,
    body: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(headers) = headers {
        for (name, value) in headers {
            if let Some(name) = name {
                builder = builder.header(name, value.clone());
            }
        }
    }
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

fn admin_router(state: AppState) -> Router {
    solo_new_api_rust::routes::admin::token::router()
        .layer(axum::Extension(AdminContext {
            user_id: 1,
            sid: "test".into(),
        }))
        .with_state(state)
}

#[tokio::test]
async fn admin_api_round_trips_rate_limits() {
    let state = state_with_pool(pool_with_tables().await).await;
    let app = admin_router(state.clone());

    let (status, body) = call(
        &app,
        "POST",
        "/api/token",
        None,
        Some(serde_json::json!({
            "name": "limited",
            "rpm": 60,
            "concurrency": 2,
        })),
    )
    .await;
    assert_eq!(status, 200, "create: {body}");
    let id = body["data"]["id"].as_i64().unwrap();
    assert_eq!(body["data"]["rpm"], 60);
    assert_eq!(body["data"]["concurrency"], 2);

    let (status, body) = call(&app, "GET", &format!("/api/token/{id}"), None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["data"]["rpm"], 60);
    assert_eq!(body["data"]["concurrency"], 2);

    let (status, body) = call(&app, "GET", "/api/token", None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["data"]["items"][0]["rpm"], 60);
    assert_eq!(body["data"]["items"][0]["concurrency"], 2);

    // 只更新名字：限流配置应原样保留。
    let (status, body) = call(
        &app,
        "PUT",
        "/api/token?id=1",
        None,
        Some(serde_json::json!({ "id": id, "name": "renamed" })),
    )
    .await;
    assert_eq!(status, 200, "update: {body}");
    assert_eq!(body["data"]["rpm"], 60);
    assert_eq!(body["data"]["concurrency"], 2);

    // 显式给 0 关闭：配置行删除，同等于不限。
    let (status, body) = call(
        &app,
        "PUT",
        "/api/token?id=1",
        None,
        Some(serde_json::json!({
            "id": id,
            "name": "renamed",
            "rpm": 0,
            "concurrency": 0,
        })),
    )
    .await;
    assert_eq!(status, 200, "clear: {body}");
    assert_eq!(body["data"]["rpm"], 0);
    assert_eq!(body["data"]["concurrency"], 0);
    assert!(state.rate_limits.get(id).is_none());
    let row: Option<(i64, i64)> =
        sqlx::query_as("SELECT rpm, concurrency FROM token_rate_limits WHERE token_id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .unwrap();
    assert!(row.is_none(), "clear should delete the row");

    // 负值直接拒绝。
    let (status, _) = call(
        &app,
        "PUT",
        "/api/token?id=1",
        None,
        Some(serde_json::json!({ "id": id, "rpm": -1 })),
    )
    .await;
    assert_eq!(status, 400);

    // 删除令牌连带清理限流配置。
    let (status, body) = call(&app, "DELETE", &format!("/api/token/{id}/"), None, None).await;
    assert_eq!(status, 200, "delete: {body}");
    let row: Option<(i64, i64)> =
        sqlx::query_as("SELECT rpm, concurrency FROM token_rate_limits WHERE token_id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .unwrap();
    assert!(row.is_none());
}

#[tokio::test]
async fn admin_create_without_limits_leaves_no_row() {
    let state = state_with_pool(pool_with_tables().await).await;
    let app = admin_router(state.clone());
    let (status, body) = call(
        &app,
        "POST",
        "/api/token",
        None,
        Some(serde_json::json!({ "name": "plain" })),
    )
    .await;
    assert_eq!(status, 200, "create: {body}");
    let id = body["data"]["id"].as_i64().unwrap();
    assert_eq!(body["data"]["rpm"], 0);
    assert_eq!(body["data"]["concurrency"], 0);
    let row: Option<(i64, i64)> =
        sqlx::query_as("SELECT rpm, concurrency FROM token_rate_limits WHERE token_id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .unwrap();
    assert!(row.is_none());
}

async fn stub_response() -> axum::response::Response {
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn relay_app(state: AppState) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(stub_response))
        .route("/v1/models", get(stub_response))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::auth::token::token_auth,
        ))
        .with_state(state)
}

fn auth_header() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer sk-gatekey123"),
    );
    headers
}

const GATE_KEY_SQL: &str = "INSERT INTO tokens (user_id, `key`, status, name, remain_quota, expired_time, unlimited_quota, model_limits_enabled, model_limits, allow_ips, `group`, auto_groups, cross_group_retry, used_quota, allow_user_quota_query, created_time, accessed_time) \
     VALUES (1, 'gatekey123', 1, 'gate', 0, -1, 1, 0, '', '', 'default', NULL, 0, 0, 1, 1, 1)";

async fn seeded_gate_state() -> AppState {
    let pool = pool_with_tables().await;
    sqlx::query(GATE_KEY_SQL).execute(&pool).await.unwrap();
    let state = state_with_pool(pool).await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM tokens WHERE `key`='gatekey123'")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    state.rate_limits.set(id, 1, 0);
    // 缩短等待预算：rpm=1 的第二个请求要补满一个令牌需近 60s，远超注入的 300ms，
    // gate 立即判超时走 429，避免集成测试真实挂 60s。
    state
        .rate_limits
        .set_max_wait(std::time::Duration::from_millis(300));
    state
}

#[tokio::test]
async fn middleware_429s_chat_when_rpm_exhausted_but_not_models_list() {
    let state = seeded_gate_state().await;
    let app = relay_app(state).await;

    let (status, body) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some(auth_header()),
        Some(serde_json::json!({ "model": "gpt-4o", "messages": [] })),
    )
    .await;
    assert_eq!(status, 200, "first request should pass: {body}");

    let (status, body) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some(auth_header()),
        Some(serde_json::json!({ "model": "gpt-4o", "messages": [] })),
    )
    .await;
    assert_eq!(status, 429, "second request should be rate limited: {body}");
    assert_eq!(body["error"]["type"], "rate_limit_error");

    // GET 不产生上游调用，不受限流影响。
    let (status, _) = call(&app, "GET", "/v1/models", Some(auth_header()), None).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn middleware_anonymous_token_still_rejected() {
    let state = seeded_gate_state().await;
    let app = relay_app(state).await;
    let mut headers = HeaderMap::new();
    headers.insert("authorization", HeaderValue::from_static("Bearer sk-wrong"));
    let (status, _) = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some(headers),
        Some(serde_json::json!({ "model": "gpt-4o", "messages": [] })),
    )
    .await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn middleware_releases_concurrency_permit_when_body_consumed() {
    let pool = pool_with_tables().await;
    sqlx::query(GATE_KEY_SQL).execute(&pool).await.unwrap();
    let state = state_with_pool(pool).await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM tokens WHERE `key`='gatekey123'")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    // 并发=1：第一个请求持有许可直到响应体被消费；消费后第二个请求应立刻放行，
    // 若许可泄漏，第二个请求会等满 60s 再 429，测试在此会明显变慢失败。
    state.rate_limits.set(id, 0, 1);
    let app = relay_app(state).await;

    let first = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some(auth_header()),
        Some(serde_json::json!({ "model": "gpt-4o", "messages": [] })),
    )
    .await;
    assert_eq!(first.0, 200, "first: {first:?}");

    let second = call(
        &app,
        "POST",
        "/v1/chat/completions",
        Some(auth_header()),
        Some(serde_json::json!({ "model": "gpt-4o", "messages": [] })),
    )
    .await;
    assert_eq!(
        second.0, 200,
        "second should pass once body released: {second:?}"
    );
}

#[test]
fn http_status_const_used() {
    assert_eq!(StatusCode::TOO_MANY_REQUESTS.as_u16(), 429);
}
