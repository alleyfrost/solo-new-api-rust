use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::routing::post;
use tower::ServiceExt;

use solo_new_api_rust::config::Config;
use solo_new_api_rust::mem::BodyGate;

fn state() -> solo_new_api_rust::state::AppState {
    let cfg = Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    solo_new_api_rust::state::AppState::new(
        cfg,
        sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap(),
        solo_new_api_rust::state::Caches {
            users: HashMap::new(),
            tokens: HashMap::new(),
            channels: HashMap::new(),
            abilities: HashMap::new(),
            options: HashMap::new(),
        },
        b"test-secret".to_vec(),
        None,
    )
}

fn big_request() -> Request {
    Request::builder()
        .method("POST")
        .uri("/upload")
        .header("content-length", (9 * 1024 * 1024).to_string())
        .body(Body::empty())
        .unwrap()
}

fn small_request() -> Request {
    Request::builder()
        .method("POST")
        .uri("/upload")
        .header("content-length", "1024")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn huge_bodies_are_serialized_and_small_bodies_are_not() {
    let app = Router::new()
        .route(
            "/upload",
            post(|| async {
                tokio::time::sleep(Duration::from_millis(120)).await;
                "ok"
            }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state(),
            solo_new_api_rust::mem::large_body_gate,
        ));

    let started = Instant::now();
    let a = app.clone().oneshot(big_request());
    let b = async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        app.clone().oneshot(big_request()).await
    };
    let _ = tokio::join!(a, b);
    assert!(
        started.elapsed() >= Duration::from_millis(220),
        "huge bodies must be serialized, elapsed {:?}",
        started.elapsed()
    );

    let started = Instant::now();
    let _ = app.clone().oneshot(small_request()).await;
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "small bodies should bypass the gate, elapsed {:?}",
        started.elapsed()
    );
}

#[test]
fn gate_limits_are_bounded() {
    let gate = BodyGate::default();
    assert_eq!(gate.big.available_permits(), 2);
    assert_eq!(gate.huge.available_permits(), 1);
}
