use std::collections::HashMap;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use solo_new_api_rust::auth::session::{Claims, issue_access};
use solo_new_api_rust::model::{AbilityRow, ChannelRow, UserRow};
use solo_new_api_rust::state::{AppState, Caches};

const SID: &str = "pg-session";

struct Fixture {
    app: Router,
    pool: SqlitePool,
    writer: tokio::task::JoinHandle<()>,
    token: String,
}

/// 两个分组（default / team）都指向同一渠道，用来验证 body.group 的选路与鉴权。
async fn setup(base_url: &str) -> Fixture {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, accessed_time INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE user_sessions (sid TEXT PRIMARY KEY, user_id INTEGER, version INTEGER, user_auth_version INTEGER, \
         status TEXT, refresh_hash TEXT, previous_refresh_hash TEXT, previous_valid_until INTEGER NOT NULL DEFAULT 0, \
         login_method TEXT, ip TEXT, user_agent TEXT, created_at INTEGER, last_active_at INTEGER, expires_at INTEGER, \
         revoked_at INTEGER DEFAULT 0, revoked_reason TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_sessions (sid, user_id, version, user_auth_version, status, refresh_hash, login_method, ip, user_agent, created_at, last_active_at, expires_at) \
         VALUES (?, 1, 1, 1, 'active', 'hash', 'password', '10.0.0.1', 'UA', 1700000000, 1700000000, 9999999999)",
    )
    .bind(SID)
    .execute(&pool)
    .await
    .unwrap();

    let mut caches = Caches {
        users: HashMap::new(),
        tokens: HashMap::new(),
        channels: HashMap::new(),
        abilities: HashMap::new(),
        options: HashMap::new(),
    };
    caches.users.insert(
        1,
        UserRow {
            id: 1,
            username: "root".into(),
            password: "x".into(),
            display_name: None,
            role: 100,
            status: 1,
            group: "default".into(),
            auth_version: 1,
            email: None,
        },
    );
    caches.channels.insert(
        3,
        ChannelRow {
            id: 3,
            r#type: 1,
            key: "sk-up".into(),
            status: 1,
            name: "c".into(),
            models: "m1".into(),
            group: "default".into(),
            base_url: Some(base_url.into()),
            priority: Some(0),
            weight: Some(0),
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
        },
    );
    for group in ["default", "team"] {
        caches.abilities.insert(
            (group.into(), "m1".into()),
            vec![AbilityRow {
                group: group.into(),
                model: "m1".into(),
                channel_id: 3,
                enabled: true,
                priority: Some(0),
                weight: Some(0),
            }],
        );
    }

    let (tx, rx) = solo_new_api_rust::logging::channel(64);
    let writer = solo_new_api_rust::logging::spawn_writer(pool.clone(), rx);
    let config = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(config, pool.clone(), caches, b"test-secret".to_vec(), Some(tx));
    let token = issue_access(
        &Claims {
            sub: 1,
            sid: SID.into(),
            ver: 1,
            uver: 1,
            iat: 1,
            exp: 9_999_999_999,
        },
        b"test-secret",
    )
    .unwrap();

    // 与 main.rs 一致：/pg 走会话认证，不带客户端 token。
    let app = Router::new()
        .route(
            "/pg/chat/completions",
            axum::routing::post(solo_new_api_rust::relay::playground::chat_completions),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::auth::admin::admin_auth,
        ))
        .with_state(state);
    Fixture {
        app,
        pool,
        writer,
        token,
    }
}

async fn send(fixture: Fixture, body: Value) -> (StatusCode, Vec<u8>, SqlitePool) {
    let Fixture {
        app,
        pool,
        writer,
        token,
    } = fixture;
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/pg/chat/completions")
                .header("authorization", format!("Bearer {token}"))
                .header("cookie", "new_api_refresh=secret-value")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    let _ = tokio::time::timeout(Duration::from_secs(3), writer)
        .await
        .expect("log writer should stop after router drop");
    (status, bytes, pool)
}

#[tokio::test]
async fn playground_forwards_without_group_or_session_cookie() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"id":"x","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let fixture = setup(&server.uri()).await;
    let (status, body, pool) = send(
        fixture,
        json!({"model": "m1", "group": "default", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("\"content\":\"hi\""));

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let sent: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert!(
        sent.get("group").is_none(),
        "选路用的 group 不应转发给上游：{sent}"
    );
    assert_eq!(reqs[0].headers["authorization"], "Bearer sk-up");
    assert!(
        !reqs[0].headers.contains_key("cookie"),
        "会话 cookie 不能泄漏到上游"
    );

    let row: (String, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT token_name, token_id, quota, prompt_tokens, completion_tokens FROM logs",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, ("playground-default".into(), 0, 0, 10, 5));
}

#[tokio::test]
async fn playground_routes_by_requested_group_and_logs_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                r#"{"id":"x","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#,
                "application/json",
            ),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri()).await;
    let (status, _, pool) =
        send(fixture, json!({"model": "m1", "group": "team", "messages": []})).await;
    assert_eq!(status, StatusCode::OK);
    let (group, token_name): (String, String) =
        sqlx::query_as("SELECT `group`, token_name FROM logs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((group, token_name), ("team".into(), "playground-team".into()));
}

#[tokio::test]
async fn playground_rejects_group_outside_routing_table() {
    let server = MockServer::start().await;
    let fixture = setup(&server.uri()).await;
    let (status, body, _) =
        send(fixture, json!({"model": "m1", "group": "vip", "messages": []})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        String::from_utf8_lossy(&body).contains("no access to group vip"),
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn playground_needs_a_session() {
    let server = MockServer::start().await;
    let fixture = setup(&server.uri()).await;
    let res = fixture
        .app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/pg/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"m1","messages":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(server.received_requests().await.unwrap().is_empty());
}
