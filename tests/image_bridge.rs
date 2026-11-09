use std::collections::HashMap;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::Request;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use solo_new_api_rust::model::{AbilityRow, ChannelRow, TokenRow, UserRow};
use solo_new_api_rust::state::{AppState, Caches};

const IMAGE_MODEL: &str = "img-model";

struct Fixture {
    app: Router,
    pool: SqlitePool,
    writer: tokio::task::JoinHandle<()>,
}

async fn setup(base_url: &str) -> Fixture {
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
            r#type: 1,
            key: "sk-up".into(),
            status: 1,
            name: "c".into(),
            models: format!("m1,{IMAGE_MODEL}"),
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
    for model in ["m1", IMAGE_MODEL] {
        caches.abilities.insert(
            ("default".into(), model.into()),
            vec![AbilityRow {
                group: "default".into(),
                model: model.into(),
                channel_id: 3,
                enabled: true,
                priority: Some(0),
                weight: Some(0),
            }],
        );
    }
    caches.options.insert("RetryTimes".into(), "1".into());
    caches
        .options
        .insert("ImageModels".into(), IMAGE_MODEL.to_string());

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
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::auth::token::token_auth,
        ))
        .with_state(state);
    Fixture { app, pool, writer }
}

async fn send(fixture: Fixture, body: Value) -> (u16, Bytes, headers::ContentType, SqlitePool) {
    let Fixture { app, pool, writer } = fixture;
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer sk-abc123")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let content_type = headers::ContentType(
        res.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string(),
    );
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let _ = tokio::time::timeout(Duration::from_secs(3), writer)
        .await
        .expect("log writer should stop after router drop");
    (status, bytes, content_type, pool)
}

mod headers {
    pub struct ContentType(pub String);
}

fn image_ok() -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(
            r#"{"created":1,"output_format":"png","data":[{"b64_json":"QUJD"}],"usage":{"input_tokens":12,"output_tokens":34,"total_tokens":46}}"#,
        )
}

#[tokio::test]
async fn chat_on_image_model_translates_to_images_and_wraps_the_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .and(body_partial_json(
            json!({"model": IMAGE_MODEL, "prompt": "画一只猫\n蓝紫色"}),
        ))
        .respond_with(image_ok())
        .mount(&server)
        .await;

    let fixture = setup(&server.uri()).await;
    let (status, body, content_type, pool) = send(
        fixture,
        json!({
            "model": IMAGE_MODEL,
            "messages": [
                {"role": "system", "content": "你是画师"},
                {"role": "user", "content": [
                    {"type": "text", "text": "画一只猫"},
                    {"type": "text", "text": "蓝紫色"}
                ]}
            ]
        }),
    )
    .await;
    let text = String::from_utf8_lossy(&body);
    assert_eq!(status, 200, "{text}");
    assert!(
        content_type.0.starts_with("application/json"),
        "{}",
        content_type.0
    );
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["object"], "chat.completion");
    assert_eq!(parsed["model"], IMAGE_MODEL);
    assert_eq!(
        parsed["choices"][0]["message"]["content"],
        "![image](data:image/png;base64,QUJD)"
    );
    assert_eq!(parsed["usage"]["prompt_tokens"], 12);
    assert_eq!(parsed["usage"]["completion_tokens"], 34);

    // 生图响应体超过 4 MiB 时 passthrough 走流式、不解析 usage，日志 token 记 0；
    // 客户端拿到的 usage 仍然是我们从包体里解析出来的。
    let logged: (String,) = sqlx::query_as("SELECT model_name FROM logs ORDER BY id DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(logged.0, IMAGE_MODEL);
}

#[tokio::test]
async fn chat_with_a_reference_image_uses_the_edits_endpoint() {
    let server = MockServer::start().await;
    let reference = "data:image/png;base64,QUJD";
    Mock::given(method("POST"))
        .and(path("/v1/images/edits"))
        .and(body_partial_json(json!({
            "model": IMAGE_MODEL,
            "prompt": "把背景换成雪山",
            "n": 1,
            "images": [{"image_url": reference}]
        })))
        .respond_with(image_ok())
        .mount(&server)
        .await;

    let fixture = setup(&server.uri()).await;
    let (status, body, _, _) = send(
        fixture,
        json!({
            "model": IMAGE_MODEL,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "把背景换成雪山"},
                {"type": "image_url", "image_url": {"url": reference}}
            ]}]
        }),
    )
    .await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    assert!(
        String::from_utf8_lossy(&body).contains("![image](data:image/png;base64,QUJD)"),
        "{}",
        String::from_utf8_lossy(&body)
    );

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let sent: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(sent["images"], json!([{"image_url": reference}]));
    assert_eq!(sent["n"], 1);
}

#[tokio::test]
async fn streaming_image_model_returns_one_sse_block() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(image_ok())
        .mount(&server)
        .await;

    let fixture = setup(&server.uri()).await;
    let (status, body, content_type, _) = send(
        fixture,
        json!({
            "model": IMAGE_MODEL,
            "stream": true,
            "messages": [{"role": "user", "content": "画一只狗"}]
        }),
    )
    .await;
    let text = String::from_utf8_lossy(&body);
    assert_eq!(status, 200);
    assert!(
        content_type.0.starts_with("text/event-stream"),
        "{}",
        content_type.0
    );
    assert!(text.contains("\"role\":\"assistant\""), "{text}");
    assert!(
        text.contains("![image](data:image/png;base64,QUJD)"),
        "{text}"
    );
    assert!(text.contains("\"finish_reason\":\"stop\""), "{text}");
    assert!(text.trim_end().ends_with("data: [DONE]"), "{text}");
}

#[tokio::test]
async fn non_image_model_keeps_the_chat_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"id":"x","choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
        ))
        .mount(&server)
        .await;

    let fixture = setup(&server.uri()).await;
    let (status, body, _, _) = send(
        fixture,
        json!({"model": "m1", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("\"content\":\"hi\""));
}

#[tokio::test]
async fn image_model_upstream_error_is_passed_through() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"error":{"message":"bad prompt"}}"#),
        )
        .mount(&server)
        .await;

    let fixture = setup(&server.uri()).await;
    let (status, body, _, _) = send(
        fixture,
        json!({"model": IMAGE_MODEL, "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, 400);
    assert!(String::from_utf8_lossy(&body).contains("bad prompt"));
}

#[tokio::test]
async fn chat_without_a_user_prompt_is_rejected_before_calling_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(image_ok())
        .mount(&server)
        .await;

    let fixture = setup(&server.uri()).await;
    let (status, body, _, _) = send(
        fixture,
        json!({"model": IMAGE_MODEL, "messages": [{"role": "system", "content": "only system"}]}),
    )
    .await;
    assert_eq!(status, 400);
    assert!(
        String::from_utf8_lossy(&body).contains("messages must contain a user prompt"),
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        server
            .received_requests()
            .await
            .map(|requests| requests.is_empty())
            .unwrap_or(false),
        "没有 prompt 时不应打到上游"
    );
}
