use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn serves_index_for_spa_routes() {
    let dir = std::env::temp_dir().join(format!("spa-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html>ok</html>").unwrap();
    let app = solo_new_api_rust::web::spa_service(dir.clone());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/keys")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("ok"));
    std::fs::remove_dir_all(dir).ok();
}
