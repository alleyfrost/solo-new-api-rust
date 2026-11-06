use axum::body::Body;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn status_returns_success() {
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: std::path::PathBuf::from(":memory:"),
        web_dist: std::path::PathBuf::from("./web/dist"),
        jwt_secret_path: std::path::PathBuf::from("/tmp/.jwt-test"),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let state = solo_new_api_rust::state::AppState { config: cfg, pool };
    let app = solo_new_api_rust::routes::public_router().with_state(state);
    let res = app.oneshot(
        axum::http::Request::builder().uri("/api/status").body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(v["data"]["self_use_mode_enabled"], true);
}
