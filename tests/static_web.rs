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
    assert_eq!(res.headers()["cache-control"], "no-cache");
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("ok"));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn hashed_static_assets_are_immutable() {
    let dir = std::env::temp_dir().join(format!("spa-static-test-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("static")).unwrap();
    std::fs::write(dir.join("static/app.js"), "console.log(1)").unwrap();
    let app = solo_new_api_rust::web::spa_service(dir.clone());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/static/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    std::fs::remove_dir_all(dir).ok();
}
