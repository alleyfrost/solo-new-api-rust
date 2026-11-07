use axum::body::Body;
use axum::extract::Request;
use axum::middleware::Next;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fake_admin(mut req: Request, next: Next) -> axum::response::Response {
    req.extensions_mut()
        .insert(solo_new_api_rust::auth::admin::AdminContext {
            user_id: 1,
            sid: "test".into(),
        });
    next.run(req).await
}

#[tokio::test]
#[ignore = "requires REAL_DB pointing at a copy of one-api.db"]
async fn real_db_schema_caches_and_token_models() {
    let path = std::env::var("REAL_DB").expect("REAL_DB");
    let pool = solo_new_api_rust::db::connect(std::path::Path::new(&path))
        .await
        .unwrap();
    solo_new_api_rust::db::validate_schema(&pool).await.unwrap();

    let caches = solo_new_api_rust::state::Caches::load(&pool).await.unwrap();
    assert!(!caches.users.is_empty());
    assert!(!caches.tokens.is_empty());
    assert!(!caches.channels.is_empty());

    let (key, user) = {
        let token = caches
            .tokens
            .values()
            .find(|t| {
                t.status == 1
                    && caches
                        .users
                        .get(&t.user_id)
                        .map(|u| u.status == 1)
                        .unwrap_or(false)
            })
            .expect("enabled token");
        let user = caches.users.get(&token.user_id).expect("token user");
        (token.key.clone(), user.clone())
    };
    assert_eq!(user.status, 1);

    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: path.into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let (tx, rx) = solo_new_api_rust::logging::channel(16);
    let state = solo_new_api_rust::state::AppState::new(
        cfg,
        pool.clone(),
        caches,
        b"test-secret".to_vec(),
        Some(tx),
    );
    let app = axum::Router::new()
        .route(
            "/v1/models",
            axum::routing::get(solo_new_api_rust::routes::models::list_models),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::auth::token::token_auth,
        ))
        .with_state(state.clone());

    let res = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header("authorization", format!("Bearer sk-{key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["success"], true);
    assert!(!v["data"]["data"].as_array().unwrap().is_empty());

    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");

    let admin = solo_new_api_rust::routes::admin::router()
        .route_layer(axum::middleware::from_fn(fake_admin))
        .with_state(state.clone());
    for uri in [
        "/api/token/?p=1&size=5",
        "/api/log?p=1&page_size=5",
        "/api/channel?p=1&page_size=5",
        "/api/models/?p=1&page_size=5",
        "/api/data?start_timestamp=0&end_timestamp=9999999999",
    ] {
        let res = admin
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&body).to_string();
        assert_eq!(status, 200, "{uri}: {text}");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["success"], true, "{uri}: {v}");
    }

    // 个人资料与会话管理只读走一遍真实 Go schema。
    let self_routes = axum::Router::new()
        .route(
            "/api/user/self",
            axum::routing::get(solo_new_api_rust::routes::user::self_info),
        )
        .route(
            "/api/user/sessions",
            axum::routing::get(solo_new_api_rust::routes::user::list_sessions),
        )
        .route_layer(axum::middleware::from_fn(fake_admin))
        .with_state(state.clone());
    for uri in ["/api/user/self", "/api/user/sessions"] {
        let res = self_routes
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&body).to_string();
        assert_eq!(status, 200, "{uri}: {text}");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["success"], true, "{uri}: {v}");
    }
    let res = self_routes
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/user/self")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let profile: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let setting = profile["data"]["setting"].as_str().unwrap_or_default();
    assert!(
        setting.is_empty() || serde_json::from_str::<serde_json::Value>(setting).is_ok(),
        "users.setting must stay valid JSON: {setting}"
    );
    assert!(profile["data"]["has_password"].is_boolean());

    let writer = solo_new_api_rust::logging::spawn_writer(pool.clone(), rx);
    let entry = solo_new_api_rust::logging::LogEntry {
        log_type: solo_new_api_rust::logging::LOG_TYPE_CONSUME,
        content: None,
        created_at: 1_800_000_000,
        user_id: user.id,
        username: user.username.clone(),
        token_name: "probe".into(),
        model_name: "compat-probe".into(),
        prompt_tokens: 3,
        completion_tokens: 2,
        use_time_secs: 1,
        is_stream: false,
        channel_id: 0,
        channel_name: "probe".into(),
        token_id: 0,
        group: "default".into(),
        ip: "127.0.0.1".into(),
        request_id: "real-db-probe".into(),
        upstream_request_id: String::new(),
        other: "{}".into(),
    };
    solo_new_api_rust::logging::enqueue(&state, entry).await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let row: (i64, i64, i64, String) = sqlx::query_as(
        "SELECT quota, prompt_tokens, completion_tokens, model_name FROM logs WHERE request_id='real-db-probe'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, (0, 3, 2, "compat-probe".to_string()));
    sqlx::query("DELETE FROM logs WHERE request_id='real-db-probe'")
        .execute(&pool)
        .await
        .unwrap();
    drop(admin);
    drop(state);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), writer).await;
}
