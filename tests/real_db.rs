use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires REAL_DB pointing at a copy of one-api.db"]
async fn real_db_schema_caches_and_token_models() {
    let path = std::env::var("REAL_DB").expect("REAL_DB");
    let pool = solo_new_api_rust::db::connect(std::path::Path::new(&path)).await.unwrap();
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
    let state = solo_new_api_rust::state::AppState {
        config: cfg,
        pool: pool.clone(),
        caches: Arc::new(RwLock::new(caches)),
        jwt_secret: Arc::new(b"test-secret".to_vec()),
    };
    let app = axum::Router::new()
        .route("/v1/models", axum::routing::get(solo_new_api_rust::routes::models::list_models))
        .layer(axum::middleware::from_fn_with_state(state.clone(), solo_new_api_rust::auth::token::token_auth))
        .with_state(state);

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
}
