use std::collections::HashMap;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use solo_new_api_rust::routes::admin::{channel, channel_ops};
use solo_new_api_rust::state::{AppState, Caches};

async fn setup() -> (Router, SqlitePool) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE channels (id INTEGER PRIMARY KEY AUTOINCREMENT, type INTEGER, `key` TEXT, \
         status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, \
         weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, \
         created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, \
         test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, \
         status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, \
         param_override TEXT, remark TEXT, settings TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER, tag TEXT, PRIMARY KEY (`group`, model, channel_id))")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)")
        .execute(&pool)
        .await
        .unwrap();

    let caches = Caches {
        users: HashMap::new(),
        tokens: HashMap::new(),
        channels: HashMap::new(),
        abilities: HashMap::new(),
        options: HashMap::new(),
    };
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(cfg, pool.clone(), caches, b"test-secret".to_vec(), None);
    let app = Router::new()
        .merge(channel::router())
        .merge(channel_ops::router())
        .with_state(state);
    (app, pool)
}

#[allow(clippy::too_many_arguments)]
async fn insert_channel(
    pool: &SqlitePool,
    id: i64,
    channel_type: i64,
    keys: &str,
    base_url: &str,
    models: &str,
    test_model: &str,
    mapping: Option<&str>,
    overrides: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO channels (id, type, `key`, status, name, models, `group`, base_url, test_model, model_mapping, header_override, response_time, test_time, auto_ban) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(channel_type)
    .bind(keys)
    .bind(1)
    .bind(format!("c{id}"))
    .bind(models)
    .bind("default")
    .bind(base_url)
    .bind(test_model)
    .bind(mapping)
    .bind(overrides)
    .bind(111_i64)
    .bind(0_i64)
    .bind(1_i64)
    .execute(pool)
    .await
    .unwrap();
}

async fn get(app: &Router, uri: &str) -> (u16, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn post_json(app: &Router, uri: &str, body: Value) -> (u16, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn chat_reply() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": "Hi" }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3 }
    }))
}

#[tokio::test]
async fn successful_test_replies_time_and_stores_timing() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer k1"))
        .respond_with(chat_reply())
        .expect(1)
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "", None, None).await;

    let (status, body) = get(&app, "/api/channel/test/3").await;
    assert_eq!(status, 200);
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    assert!(body["time"].as_f64().unwrap_or_default() >= 0.0);

    let row: (i64, i64) = sqlx::query_as("SELECT response_time, test_time FROM channels WHERE id=3")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(row.0 >= 0, "response_time should be overwritten");
    assert!(row.1 > 0, "test_time should be stamped");
    mock.verify().await;
}

#[tokio::test]
async fn model_query_param_overrides_channel_default() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat_reply())
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "cheap", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3?model=gpt-x").await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    let req = mock.received_requests().await.unwrap().pop().unwrap();
    let sent: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(sent["model"], Value::String("gpt-x".into()));
}

#[tokio::test]
async fn model_mapping_applies_to_test_probe() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat_reply())
        .mount(&mock)
        .await;
    insert_channel(
        &pool,
        3,
        1,
        "k1",
        &mock.uri(),
        "alias",
        "",
        Some(r#"{"alias":"real-model"}"#),
        None,
    )
    .await;

    let (_, body) = get(&app, "/api/channel/test/3").await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    let req = mock.received_requests().await.unwrap().pop().unwrap();
    let sent: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(sent["model"], Value::String("real-model".into()));
}

#[tokio::test]
async fn upstream_error_reports_failure_and_keeps_channel_enabled() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "message": "Incorrect API key provided", "type": "invalid_request_error", "code": "invalid_api_key" }
        })))
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "", None, None).await;

    let (status, body) = get(&app, "/api/channel/test/3").await;
    assert_eq!(status, 200, "test failures stay HTTP 200 like upstream new-api");
    assert_eq!(body["success"], Value::Bool(false));
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Incorrect API key"),
        "{body}"
    );
    assert_eq!(body["error_code"], Value::String("invalid_api_key".into()));
    assert_eq!(body["time"], Value::from(0.0));

    let status: (i64,) = sqlx::query_as("SELECT status FROM channels WHERE id=3")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status.0, 1, "a manual test must not auto-disable the channel");
    let timing: (i64, i64) = sqlx::query_as("SELECT response_time, test_time FROM channels WHERE id=3")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(timing, (111, 0), "failed test must not overwrite timings");
}

#[tokio::test]
async fn empty_reply_counts_as_failure() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-2",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }]
        })))
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3").await;
    assert_eq!(body["success"], Value::Bool(false), "{body}");
    assert_eq!(body["error_code"], Value::String("empty_reply".into()));
}

#[tokio::test]
async fn stream_probe_succeeds_on_first_event() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"H\"}}]}\n\n\
                     data: {\"choices\":[{\"delta\":{\"content\":\"i\"}}]}\n\n\
                     data: [DONE]\n\n",
                ),
        )
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3?stream=true").await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    let req = mock.received_requests().await.unwrap().pop().unwrap();
    let sent: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(sent["stream"], Value::Bool(true));
}

#[tokio::test]
async fn stream_probe_fails_when_upstream_sends_nothing() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(""),
        )
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3?stream=true").await;
    assert_eq!(body["success"], Value::Bool(false), "{body}");
}

#[tokio::test]
async fn anthropic_endpoint_uses_messages_path_and_headers() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "k1"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "Hi" }],
            "usage": { "input_tokens": 1, "output_tokens": 2 }
        })))
        .expect(1)
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "claude-3-5-sonnet", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3?endpoint_type=anthropic").await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    mock.verify().await;
}

#[tokio::test]
async fn responses_endpoint_uses_responses_path() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "output_text": "Hi",
            "output": [{ "type": "message" }]
        })))
        .expect(1)
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "gpt-4o", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3?endpoint_type=openai-response").await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    mock.verify().await;
}

#[tokio::test]
async fn unsupported_endpoint_type_is_reported_without_request() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3?endpoint_type=gemini").await;
    assert_eq!(body["success"], Value::Bool(false));
    assert_eq!(
        body["error_code"],
        Value::String("unsupported_endpoint_type".into())
    );
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn channel_without_model_is_not_tested() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3").await;
    assert_eq!(body["success"], Value::Bool(false));
    assert_eq!(body["error_code"], Value::String("no_test_model".into()));
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn multi_key_test_stops_at_first_success() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer bad"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": { "message": "rate limited", "code": "rate_limit_exceeded" }
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer good"))
        .respond_with(chat_reply())
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "bad\ngood", &mock.uri(), "m1", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3").await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    let seen = mock.received_requests().await.unwrap();
    assert_eq!(seen.len(), 2, "both keys tried in order");
}

#[tokio::test]
async fn multi_key_failure_names_the_failing_key() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "message": "bad key", "code": "invalid_api_key" }
        })))
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1\nk2", &mock.uri(), "m1", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/test/3").await;
    assert_eq!(body["success"], Value::Bool(false));
    let message = body["message"].as_str().unwrap_or_default();
    assert!(message.contains("key 2/2"), "{message}");
}

#[tokio::test]
async fn header_override_reaches_the_probe() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("x-custom", "v"))
        .respond_with(chat_reply())
        .expect(1)
        .mount(&mock)
        .await;
    insert_channel(
        &pool,
        3,
        1,
        "k1",
        &mock.uri(),
        "m1",
        "",
        None,
        Some(r#"{"X-Custom":"v"}"#),
    )
    .await;

    let (_, body) = get(&app, "/api/channel/test/3").await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    mock.verify().await;
}

#[tokio::test]
async fn missing_channel_returns_not_found() {
    let (app, _pool) = setup().await;
    let (status, body) = get(&app, "/api/channel/test/99").await;
    assert_eq!(status, 404);
    assert_eq!(body["success"], Value::Bool(false));

    let (status, _) = get(&app, "/api/channel/fetch_models/99").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn fetch_models_by_id_returns_sorted_ids() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer k1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": [{ "id": "z-model" }, { "id": "a-model" }, { "id": "a-model" }]
        })))
        .expect(1)
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "", None, None).await;

    let (status, body) = get(&app, "/api/channel/fetch_models/3").await;
    assert_eq!(status, 200);
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    assert_eq!(
        body["data"],
        Value::Array(vec![
            Value::String("a-model".into()),
            Value::String("z-model".into())
        ])
    );
}

#[tokio::test]
async fn fetch_models_error_keeps_channel_key_out_of_message() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error": { "message": "model list not allowed" }
        })))
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "supersecretkey", &mock.uri(), "m1", "", None, None).await;

    let (_, body) = get(&app, "/api/channel/fetch_models/3").await;
    assert_eq!(body["success"], Value::Bool(false), "{body}");
    let message = body["message"].as_str().unwrap_or_default();
    assert!(message.contains("model list not allowed"), "{message}");
    assert!(!message.contains("supersecretkey"), "{message}");
}

#[tokio::test]
async fn fetch_models_post_uses_form_configuration() {
    let (app, _pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer form-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{ "id": "m-a" }]
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let (_, body) = post_json(
        &app,
        "/api/channel/fetch_models",
        serde_json::json!({
            "base_url": format!("{}/v1", mock.uri()),
            "type": 1,
            "key": "form-key\nsecond-key"
        }),
    )
    .await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    assert_eq!(body["data"][0], Value::String("m-a".into()));
}

#[tokio::test]
async fn fetch_models_post_with_channel_id_uses_stored_key() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer stored"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{ "id": "m-b" }]
        })))
        .expect(1)
        .mount(&mock)
        .await;
    insert_channel(&pool, 7, 1, "stored", &mock.uri(), "m1", "", None, None).await;

    let (_, body) = post_json(
        &app,
        "/api/channel/fetch_models",
        serde_json::json!({ "channel_id": 7, "type": 1 }),
    )
    .await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    assert_eq!(body["data"][0], Value::String("m-b".into()));
}

#[tokio::test]
async fn fetch_models_post_requires_type_or_channel() {
    let (app, _pool) = setup().await;
    let (_, body) = post_json(
        &app,
        "/api/channel/fetch_models",
        serde_json::json!({ "base_url": "http://127.0.0.1:1" }),
    )
    .await;
    assert_eq!(body["success"], Value::Bool(false));
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("type is required"),
        "{body}"
    );
}

#[tokio::test]
async fn fetch_models_tolerates_a_slow_upstream() {
    let (app, pool) = setup().await;
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(1200))
                .set_body_json(serde_json::json!({ "data": [] })),
        )
        .mount(&mock)
        .await;
    insert_channel(&pool, 3, 1, "k1", &mock.uri(), "m1", "", None, None).await;

    let started = std::time::Instant::now();
    let (_, body) = get(&app, "/api/channel/fetch_models/3").await;
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// 上游持续不断下发 SSE 时，流式探测必须在第一个事件后立刻断开，
/// 不能把整条流读完（内存红线）。
#[tokio::test]
async fn stream_probe_disconnects_while_upstream_keeps_sending() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let _handle = tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0_u8; 4096];
                let _ = socket.read(&mut buf).await;
                if socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                // 4096 个事件约 16 秒：探测端若不取消，请求会一直等到超时。
                for _ in 0..4096 {
                    let event = b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
                    let mut frame = format!("{:x}\r\n", event.len()).into_bytes();
                    frame.extend_from_slice(event);
                    frame.extend_from_slice(b"\r\n");
                    if socket.write_all(&frame).await.is_err() {
                        return;
                    }
                    let _ = socket.flush().await;
                    tokio::time::sleep(Duration::from_millis(4)).await;
                }
            });
        }
    });

    let (app, pool) = setup().await;
    insert_channel(&pool, 3, 1, "k1", &format!("http://{addr}"), "m1", "", None, None).await;

    let started = std::time::Instant::now();
    let (_, body) = get(&app, "/api/channel/test/3?stream=true").await;
    let elapsed = started.elapsed();
    assert_eq!(body["success"], Value::Bool(true), "{body}");
    assert!(
        elapsed < Duration::from_secs(5),
        "probe waited {elapsed:?} for a stream that never ends"
    );
}
