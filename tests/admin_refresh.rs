use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use std::collections::HashMap;
use tower::ServiceExt;

#[tokio::test]
async fn refresh_rotates_token_then_logout_revokes() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, setting TEXT, created_at INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE user_sessions (sid TEXT PRIMARY KEY, user_id INTEGER, version INTEGER, user_auth_version INTEGER, status TEXT, refresh_hash TEXT, previous_refresh_hash TEXT, previous_valid_until INTEGER NOT NULL DEFAULT 0, login_method TEXT, ip TEXT, user_agent TEXT, created_at INTEGER, last_active_at INTEGER, expires_at INTEGER, revoked_at INTEGER DEFAULT 0, revoked_reason TEXT)").execute(&pool).await.unwrap();
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2::Argon2::default()
        .hash_password(b"secret123", &salt).unwrap().to_string();
    sqlx::query("INSERT INTO users (id,username,password,display_name,role,status,`group`,auth_version,email) VALUES (1,'alley',?,'Root',100,1,'default',1,NULL)")
        .bind(&hash).execute(&pool).await.unwrap();

    let user = solo_new_api_rust::model::UserRow {
        id: 1, username: "alley".into(), password: hash, display_name: Some("Root".into()),
        role: 100, status: 1, group: "default".into(), auth_version: 1, email: None,
    };
    let mut caches = solo_new_api_rust::state::Caches {
        users: HashMap::new(), tokens: HashMap::new(), channels: HashMap::new(),
        abilities: HashMap::new(), options: HashMap::new(),
    };
    caches.users.insert(1, user);
    let cfg = solo_new_api_rust::config::Config {
        port: 0, db_path: ":memory:".into(), web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(), trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = solo_new_api_rust::state::AppState::new(
        cfg, pool, caches, b"test-secret".to_vec(), None,
    );
    let login = axum::Router::new()
        .route("/api/user/login", axum::routing::post(solo_new_api_rust::routes::user::login))
        .route("/api/user/auth/refresh", axum::routing::post(solo_new_api_rust::routes::user::refresh))
        .route("/api/user/auth/logout", axum::routing::post(solo_new_api_rust::routes::user::logout));
    let guarded = axum::Router::new()
        .route("/api/user/self", axum::routing::get(solo_new_api_rust::routes::user::self_info))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), solo_new_api_rust::auth::admin::admin_auth));
    let app = login.merge(guarded).with_state(state);

    let res = app.clone().oneshot(Request::builder().method("POST").uri("/api/user/login")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"username":"alley","password":"secret123"}"#)).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let cookies: Vec<String> = res.headers().get_all("set-cookie")
        .iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert!(cookies.iter().any(|c| c.starts_with("new_api_has_session=1;")), "{cookies:?}");
    let cookie = cookies.iter().find(|c| c.starts_with("new_api_refresh="))
        .expect("refresh cookie").split(';').next().unwrap().to_string();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let token = v["data"]["access_token"].as_str().unwrap().to_string();
    assert!(v["data"]["session"]["sid"].as_str().is_some());

    // JWT 的 iat/exp 只有秒级精度，等待 1s 确保刷新后的 access_token 必然变化。
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let res = app.clone().oneshot(Request::builder().method("POST").uri("/api/user/auth/refresh")
        .header("cookie", cookie.clone()).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let cookies: Vec<String> = res.headers().get_all("set-cookie")
        .iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert!(cookies.iter().any(|c| c.starts_with("new_api_has_session=1;")), "{cookies:?}");
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let token2 = v["data"]["access_token"].as_str().unwrap().to_string();
    assert_ne!(token2, token);

    let res = app.clone().oneshot(Request::builder().method("POST").uri("/api/user/auth/logout")
        .header("cookie", cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let cookies: Vec<String> = res.headers().get_all("set-cookie")
        .iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert!(cookies.iter().any(|c| c.starts_with("new_api_has_session=;")), "{cookies:?}");

    let res = app.oneshot(Request::builder().uri("/api/user/self")
        .header("authorization", format!("Bearer {token2}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 401);
}
