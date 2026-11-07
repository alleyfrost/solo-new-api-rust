use std::collections::HashMap;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::Request;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use solo_new_api_rust::model::{AbilityRow, ChannelRow, TokenRow, UserRow};
use solo_new_api_rust::state::{AppState, Caches};

struct Fixture {
    app: Router,
    pool: SqlitePool,
    writer: tokio::task::JoinHandle<()>,
}

async fn setup(
    base_url: &str,
    keys: &str,
    channel_type: i64,
    mapping: Option<&str>,
    overrides: Option<&str>,
) -> Fixture {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, accessed_time INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO tokens (id, accessed_time) VALUES (2, 0)")
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
    caches.tokens.insert(
        "abc123".into(),
        TokenRow {
            id: 2,
            user_id: 1,
            key: "abc123".into(),
            status: 1,
            name: "t".into(),
            expired_time: -1,
            remain_quota: 0,
            unlimited_quota: true,
            model_limits_enabled: false,
            model_limits: String::new(),
            allow_ips: None,
            group: "default".into(),
            auto_groups: None,
            cross_group_retry: false,
            created_time: None,
            accessed_time: None,
            used_quota: None,
            allow_user_quota_query: None,
        },
    );
    caches.channels.insert(
        3,
        ChannelRow {
            id: 3,
            r#type: channel_type,
            key: keys.into(),
            status: 1,
            name: "c".into(),
            models: "m1".into(),
            group: "default".into(),
            base_url: Some(base_url.into()),
            priority: Some(0),
            weight: Some(0),
            channel_info: None,
            model_mapping: mapping.map(str::to_string),
            header_override: overrides.map(str::to_string),
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
    caches.abilities.insert(
        ("default".into(), "m1".into()),
        vec![AbilityRow {
            group: "default".into(),
            model: "m1".into(),
            channel_id: 3,
            enabled: true,
            priority: Some(0),
            weight: Some(0),
        }],
    );
    caches.options.insert("RetryTimes".into(), "3".into());

    let (tx, rx) = solo_new_api_rust::logging::channel(64);
    let writer = solo_new_api_rust::logging::spawn_writer(pool.clone(), rx);
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(cfg, pool.clone(), caches, b"test-secret".to_vec(), Some(tx));
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            axum::routing::post(solo_new_api_rust::relay::chat::chat_completions),
        )
        .route(
            "/v1/messages",
            axum::routing::post(solo_new_api_rust::relay::messages::messages),
        )
        .route(
            "/v1/responses",
            axum::routing::post(solo_new_api_rust::relay::responses::responses),
        )
        .route(
            "/v1/embeddings",
            axum::routing::post(solo_new_api_rust::relay::extra::embeddings),
        )
        .route(
            "/v1/audio/transcriptions",
            axum::routing::post(solo_new_api_rust::relay::extra::audio_transcriptions),
        )
        .route(
            "/v1/audio/speech",
            axum::routing::post(solo_new_api_rust::relay::extra::audio_speech),
        )
        .route(
            "/v1/rerank",
            axum::routing::post(solo_new_api_rust::relay::extra::rerank),
        )
        .route(
            "/v1/completions",
            axum::routing::post(solo_new_api_rust::relay::extra::completions),
        )
        .route(
            "/v1/moderations",
            axum::routing::post(solo_new_api_rust::relay::extra::moderations),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::auth::token::token_auth,
        ))
        .with_state(state);
    Fixture { app, pool, writer }
}

async fn send(fixture: Fixture, body: &str) -> (u16, Bytes, SqlitePool) {
    send_to(fixture, "/v1/chat/completions", body).await
}

async fn send_to(fixture: Fixture, path: &str, body: &str) -> (u16, Bytes, SqlitePool) {
    let Fixture { app, pool, writer } = fixture;
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", "Bearer sk-abc123")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let _ = tokio::time::timeout(Duration::from_secs(3), writer)
        .await
        .expect("log writer should stop after router drop");
    (status, bytes, pool)
}

fn ok_json() -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(
            r#"{"id":"x","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#,
        )
}

#[tokio::test]
async fn non_stream_forwards_and_logs_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-up"))
        .respond_with(ok_json())
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, pool) = send(
        fixture,
        r#"{"model":"m1","messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("\"content\":\"hi\""));

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(sent["model"], "m1");
    assert_eq!(reqs[0].headers["authorization"], "Bearer sk-up");

    let row: (i64, i64, i64, i64, String, i64) = sqlx::query_as(
        "SELECT COUNT(*), SUM(quota), SUM(prompt_tokens), SUM(completion_tokens), MAX(model_name), MAX(is_stream) FROM logs",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, (1, 0, 10, 5, "m1".into(), 0));
}

#[tokio::test]
async fn mapping_and_header_override_apply_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_json())
        .mount(&server)
        .await;

    let fixture = setup(
        &server.uri(),
        "sk-up",
        1,
        Some(r#"{"m1":"up-m1"}"#),
        Some(r#"{"X-Custom":"v"}"#),
    )
    .await;
    let (status, _, _) = send(fixture, r#"{"model":"m1","messages":[]}"#).await;
    assert_eq!(status, 200);

    let reqs = server.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(sent["model"], "up-m1");
    assert_eq!(reqs[0].headers["x-custom"], "v");
}

#[tokio::test]
async fn rate_limit_rotates_key_and_retries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer k1"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer k2"))
        .respond_with(ok_json())
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "k1\nk2", 1, None, None).await;
    let (status, _, _) = send(fixture, r#"{"model":"m1","messages":[]}"#).await;
    assert_eq!(status, 200);
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].headers["authorization"], "Bearer k1");
    assert_eq!(reqs[1].headers["authorization"], "Bearer k2");
}

#[tokio::test]
async fn rate_limit_is_not_treated_as_channel_outage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"error":{"message":"inference exceeds tpm/rpm limit"}}"#),
        )
        .mount(&server)
        .await;

    let Fixture { app, pool, writer } = setup(&server.uri(), "k1", 1, None, None).await;
    // 同一个进程内连续两次：若 429 把唯一 Key 冷却十分钟，第二次会变成 503。
    for _ in 0..2 {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer sk-abc123")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"m1","messages":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 429);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains("tpm/rpm limit"));
    }
    drop(app);
    let _ = tokio::time::timeout(Duration::from_secs(3), writer).await;
    // RetryTimes=3：每次请求 4 次上游尝试，两次请求都应真正打到上游。
    assert_eq!(server.received_requests().await.unwrap().len(), 8);
    let rows: (i64, String) = sqlx::query_as(
        "SELECT COUNT(*), MAX(content) FROM logs WHERE type=5",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows.0, 2, "失败请求也应写入使用记录");
    assert!(rows.1.contains("status_code=429"));
}

#[tokio::test]
async fn cached_tokens_are_recorded_in_log_other() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(
                    r#"{"id":"x","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":6}}}"#,
                ),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, _, pool) = send(fixture, r#"{"model":"m1","messages":[]}"#).await;
    assert_eq!(status, 200);
    let other: String =
        sqlx::query_scalar("SELECT other FROM logs WHERE type=2 ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let v: serde_json::Value = serde_json::from_str(&other).unwrap();
    assert_eq!(v["cache_tokens"], 6);
}

#[tokio::test]
async fn client_error_is_not_retried_and_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"error":{"message":"bad request"}}"#),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, _) = send(fixture, r#"{"model":"m1","messages":[]}"#).await;
    assert_eq!(status, 400);
    assert!(String::from_utf8_lossy(&body).contains("bad request"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn stream_tap_hides_usage_and_logs_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"id\":\"x\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                     data: {\"id\":\"x\",\"choices\":[],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3}}\n\n\
                     data: [DONE]\n\n",
                ),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, pool) = send(fixture, r#"{"model":"m1","messages":[],"stream":true}"#).await;
    assert_eq!(status, 200);
    let text = String::from_utf8_lossy(&body).to_string();
    assert!(text.contains("\"content\":\"hi\""));
    assert!(
        !text.contains("prompt_tokens"),
        "未要求 usage 时必须隐藏 usage-only 事件"
    );
    assert!(text.contains("[DONE]"));

    let reqs = server.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(sent["stream_options"]["include_usage"], true);

    let (count, pt, ct, is_stream): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), SUM(prompt_tokens), SUM(completion_tokens), MAX(is_stream) FROM logs",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((count, pt, ct, is_stream), (1, 2, 3, 1));
}

#[tokio::test]
async fn stream_keeps_usage_when_client_requests_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"id\":\"x\",\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n\
                     data: [DONE]\n\n",
                ),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, _) = send(
        fixture,
        r#"{"model":"m1","messages":[],"stream":true,"stream_options":{"include_usage":true}}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("prompt_tokens"));
}

#[tokio::test]
async fn anthropic_client_translates_to_chat_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ok_json())
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, pool) = send_to(
        fixture,
        "/v1/messages",
        r#"{"model":"m1","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["content"][0]["type"], "text");
    assert_eq!(v["content"][0]["text"], "hi");

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(sent["model"], "m1");
    assert_eq!(sent["messages"][0]["content"], "hi");

    let (pt, ct): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0) FROM logs",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((pt, ct), (10, 5));
}

#[tokio::test]
async fn anthropic_stream_translates_from_chat_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"id\":\"x\",\"model\":\"m1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hi\"},\"finish_reason\":null}]}\n\n\
                     data: {\"id\":\"x\",\"model\":\"m1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2}}\n\n\
                     data: [DONE]\n\n",
                ),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, pool) = send_to(
        fixture,
        "/v1/messages",
        r#"{"model":"m1","max_tokens":100,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    assert_eq!(status, 200);
    let text = String::from_utf8_lossy(&body).to_string();
    assert!(text.contains("event: message_start"), "{text}");
    assert!(text.contains("event: content_block_delta"), "{text}");
    assert!(text.contains("event: message_stop"), "{text}");
    assert!(text.contains("\"text\":\"hi\""), "{text}");

    let (pt, ct): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0) FROM logs",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((pt, ct), (1, 2));
}

#[tokio::test]
async fn responses_client_translates_to_chat_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_json())
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, _) =
        send_to(fixture, "/v1/responses", r#"{"model":"m1","input":"hi"}"#).await;
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let text: String = v["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"].as_array())
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .collect();
    assert_eq!(text, "hi");
    assert_eq!(v["usage"]["input_tokens"], 10);
    assert_eq!(v["usage"]["output_tokens"], 5);
}

#[tokio::test]
async fn anthropic_upstream_passthrough_uses_x_api_key() {
    let server = MockServer::start().await;
    let anthropic_body = r#"{"id":"msg_1","type":"message","role":"assistant","model":"m1","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":4}}"#;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(anthropic_body),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 14, None, None).await;
    let (status, body, _) = send_to(
        fixture,
        "/v1/messages",
        r#"{"model":"m1","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body, Bytes::from(anthropic_body));

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs[0].headers["x-api-key"], "sk-up");
    assert!(!reqs[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn chat_client_translates_to_anthropic_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(
                    r#"{"id":"msg_1","type":"message","role":"assistant","model":"m1","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":4}}"#,
                ),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 14, None, None).await;
    let (status, body, _) = send(
        fixture,
        r#"{"model":"m1","messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "hi");
    assert_eq!(v["usage"]["prompt_tokens"], 3);
    assert_eq!(v["usage"]["completion_tokens"], 4);

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs[0].headers["x-api-key"], "sk-up");
    let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(sent["model"], "m1");
}

async fn send_raw(
    fixture: Fixture,
    path: &str,
    content_type: &str,
    body: Vec<u8>,
) -> (u16, Bytes, String, SqlitePool) {
    let Fixture { app, pool, writer } = fixture;
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", "Bearer sk-abc123")
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let ct = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
        .into_bytes();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let _ = tokio::time::timeout(Duration::from_secs(3), writer)
        .await
        .expect("log writer should stop after router drop");
    (
        status,
        bytes,
        String::from_utf8(ct).unwrap_or_default(),
        pool,
    )
}

#[tokio::test]
async fn embeddings_passthrough_preserves_body_and_logs_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .and(header("authorization", "Bearer sk-up"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1]}],"model":"m1","usage":{"prompt_tokens":7,"total_tokens":7}}"#,
                "application/json",
            ),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, ct, pool) = send_raw(
        fixture,
        "/v1/embeddings",
        "application/json",
        br#"{"model":"m1","input":"hello"}"#.to_vec(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(ct, "application/json");
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["data"][0]["embedding"][0], 0.1);

    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].headers["content-type"].to_str().unwrap(),
        "application/json"
    );
    let req: serde_json::Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(req["input"], "hello");
    assert_eq!(req["model"], "m1");

    let row: (i64, i64, i64, String) = sqlx::query_as(
        "SELECT prompt_tokens, completion_tokens, quota, model_name FROM logs ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, (7, 0, 0, "m1".to_string()));
}

#[tokio::test]
async fn audio_transcription_uses_multipart_model_for_routing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"text":"hello world","usage":{"prompt_tokens":5,"completion_tokens":2}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let boundary = "----probe";
    let multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nm1\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFDATA\r\n--{boundary}--\r\n"
    )
    .into_bytes();
    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, _ct, _pool) = send_raw(
        fixture,
        "/v1/audio/transcriptions",
        &format!("multipart/form-data; boundary={boundary}"),
        multipart,
    )
    .await;
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["text"], "hello world");

    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].headers["content-type"]
            .to_str()
            .unwrap()
            .contains(&boundary)
    );
    let upstream_body = String::from_utf8_lossy(&sent[0].body);
    assert!(upstream_body.contains("name=\"model\""));
    assert!(upstream_body.contains("\r\n\r\nm1\r\n"));
    assert!(upstream_body.contains("RIFFDATA"));
}

#[tokio::test]
async fn audio_speech_streams_binary_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(b"MP3DATA".to_vec(), "audio/mpeg"))
        .mount(&server)
        .await;

    let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
    let (status, body, ct, _pool) = send_raw(
        fixture,
        "/v1/audio/speech",
        "application/json",
        br#"{"model":"m1","input":"hi","voice":"alloy"}"#.to_vec(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(ct, "audio/mpeg");
    assert_eq!(&body[..], b"MP3DATA");
}

#[tokio::test]
async fn completions_rerank_moderations_share_passthrough() {
    for endpoint in ["/v1/completions", "/v1/rerank", "/v1/moderations"] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"id":"x","usage":{"prompt_tokens":1,"completion_tokens":2}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let fixture = setup(&server.uri(), "sk-up", 1, None, None).await;
        let (status, body, _ct, _pool) = send_raw(
            fixture,
            endpoint,
            "application/json",
            br#"{"model":"m1","prompt":"hi"}"#.to_vec(),
        )
        .await;
        assert_eq!(status, 200, "{endpoint}");
        assert!(String::from_utf8_lossy(&body).contains("\"id\":\"x\""));
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "{endpoint}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_all_succeed_and_log() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ok_json())
        .mount(&server)
        .await;

    let Fixture { app, pool, writer } = setup(&server.uri(), "sk-up", 1, None, None).await;
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            let res = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/chat/completions")
                        .header("authorization", "Bearer sk-abc123")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            r#"{"model":"m1","messages":[{"role":"user","content":"hi"}]}"#,
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            res.status().as_u16()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), 200);
    }
    drop(app);
    let _ = tokio::time::timeout(Duration::from_secs(5), writer).await;
    let (count, prompt, completion): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0) FROM logs",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 32);
    assert_eq!(prompt, 320);
    assert_eq!(completion, 160);
}
