use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

async fn state_with(disable_login: bool) -> solo_new_api_rust::state::AppState {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, setting TEXT, created_at INTEGER, deleted_at TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (id,username,password,display_name,role,status,`group`,auth_version,email) VALUES (1,'root','x','root',100,1,'default',1,NULL)")
        .execute(&pool)
        .await
        .unwrap();
    let user = solo_new_api_rust::model::UserRow {
        id: 1,
        username: "root".into(),
        password: "x".into(),
        display_name: Some("root".into()),
        role: 100,
        status: 1,
        group: "default".into(),
        auth_version: 1,
        email: None,
    };
    let mut caches = solo_new_api_rust::state::Caches {
        users: HashMap::new(),
        tokens: HashMap::new(),
        channels: HashMap::new(),
        abilities: HashMap::new(),
        options: HashMap::new(),
    };
    caches.users.insert(1, user);
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-desktop-test".into(),
        trust_proxy_headers: false,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let mut state =
        solo_new_api_rust::state::AppState::new(cfg, pool, caches, b"test-secret".to_vec(), None);
    state.disable_login = disable_login;
    state
}

fn app(state: solo_new_api_rust::state::AppState) -> axum::Router {
    let guarded = axum::Router::new()
        .route(
            "/api/user/self",
            axum::routing::get(solo_new_api_rust::routes::user::self_info),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::auth::admin::admin_auth,
        ));
    axum::Router::new()
        .route(
            "/api/user/auth/refresh",
            axum::routing::post(solo_new_api_rust::routes::user::refresh),
        )
        .merge(guarded)
        .with_state(state)
}

#[tokio::test]
async fn desktop_mode_skips_login_but_regular_mode_does_not() {
    let desktop = app(state_with(true).await);
    let res = desktop
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/user/auth/refresh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["data"]["user"]["username"], "root");
    assert_eq!(value["data"]["access_token"], "desktop-session");

    let res = desktop
        .oneshot(
            Request::builder()
                .uri("/api/user/self")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let regular = app(state_with(false).await);
    let res = regular
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/user/auth/refresh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let res = regular
        .oneshot(
            Request::builder()
                .uri("/api/user/self")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
