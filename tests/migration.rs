use std::collections::HashMap;

use axum::body::Body;
use axum::extract::Request;
use axum::http::Request as HttpRequest;
use axum::http::header::CONTENT_TYPE;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::auth::admin::AdminContext;
use solo_new_api_rust::config::Config;
use solo_new_api_rust::state::{AppState, Caches};

async fn inject_session(
    mut req: Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, solo_new_api_rust::error::ApiError> {
    req.extensions_mut().insert(AdminContext {
        user_id: 1,
        sid: "test-sid".into(),
    });
    Ok(next.run(req).await)
}

#[tokio::test]
async fn migration_strips_audit_metadata_and_backs_up() {
    let dir = std::env::temp_dir().join(format!("migrate-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("one-api.db");
    let pool = SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE quota_data (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,username TEXT DEFAULT '',model_name TEXT DEFAULT '',created_at INTEGER,use_group TEXT DEFAULT '',token_id INTEGER DEFAULT 0,channel_id INTEGER DEFAULT 0,node_name TEXT DEFAULT '',token_used INTEGER DEFAULT 0,count INTEGER DEFAULT 0,quota INTEGER DEFAULT 0,cache_read_tokens INTEGER DEFAULT 0,cache_creation_tokens INTEGER DEFAULT 0)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO logs (user_id,created_at,type,username,model_name,quota,prompt_tokens,completion_tokens,channel_id,token_id,`group`,other) VALUES (1,100,2,'root','m1',0,10,5,3,2,'default','{\"cache_tokens\":5,\"admin_info\":{\"use_channel\":[\"3\"],\"request_policy\":[{\"attempt\":1}]},\"expr_b64\":\"abc\",\"model_ratio\":1}')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO logs (user_id,created_at,type,username,model_name,quota,other) VALUES (1,101,5,'root','m1',0,'{\"error_code\":\"x\",\"admin_info\":{\"request_policy\":[{\"attempt\":1}]}}')")
        .execute(&pool)
        .await
        .unwrap();
    let config = Config {
        port: 0,
        db_path: db_path.clone(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(
        config,
        pool.clone(),
        Caches {
            users: HashMap::new(),
            tokens: HashMap::new(),
            channels: HashMap::new(),
            abilities: HashMap::new(),
            options: HashMap::new(),
        },
        b"test-secret".to_vec(),
        None,
    );
    let app = solo_new_api_rust::routes::admin::migrate::router()
        .route_layer(axum::middleware::from_fn(inject_session))
        .with_state(state);

    let res = app
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/api/migrate/new-api")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"backup":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["success"], true, "{v}");
    assert_eq!(v["data"]["stripped_rows"], 2);

    let other: String = sqlx::query_scalar("SELECT other FROM logs WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(other.contains("cache_tokens"));
    assert!(!other.contains("request_policy"));
    assert!(!other.contains("expr_b64"));
    let quota: (i64, i64) = sqlx::query_as("SELECT count, token_used FROM quota_data")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(quota, (1, 15));
    let backup = v["data"]["backup_path"].as_str().unwrap();
    assert!(
        std::path::Path::new(backup).is_file(),
        "backup={backup} dir={:?}",
        std::fs::read_dir(dir.parent().unwrap()).map(|it| it
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect::<Vec<_>>())
    );
    std::fs::remove_dir_all(dir).ok();
}
