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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn huge_bodies_queue_beyond_the_permit_limit_and_small_bodies_bypass() {
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

    // 4 个并发闸门：前 4 个并行完成，第 5 个必须等一轮。
    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..5 {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let _ = app.oneshot(big_request()).await;
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "the fifth huge body must wait for a permit, elapsed {:?}",
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
fn gate_limits_come_from_settings() {
    let gate = BodyGate::default();
    assert_eq!(gate.big.limit(), 8);
    assert_eq!(gate.huge.limit(), 4);
    gate.set_limits(2, 1);
    assert_eq!(gate.big.limit(), 2);
    assert_eq!(gate.huge.limit(), 1);
}

#[tokio::test]
async fn lowering_the_limit_queues_new_requests() {
    let gate = BodyGate::default();
    gate.set_limits(8, 1);
    let held = gate.huge.acquire().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), gate.huge.acquire())
            .await
            .is_err(),
        "the second request must wait while the single slot is taken"
    );
    drop(held);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), gate.huge.acquire())
            .await
            .is_ok(),
        "releasing the slot must wake the waiter"
    );
}
