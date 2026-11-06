use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

#[tokio::test]
async fn login_then_self() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE user_sessions (sid TEXT PRIMARY KEY, user_id INTEGER, version INTEGER, user_auth_version INTEGER, status TEXT, refresh_hash TEXT, login_method TEXT, ip TEXT, user_agent TEXT, created_at INTEGER, last_active_at INTEGER, expires_at INTEGER, revoked_at INTEGER DEFAULT 0)").execute(&pool).await.unwrap();
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
    let state = solo_new_api_rust::state::AppState {
        config: cfg, pool, caches: Arc::new(RwLock::new(caches)),
        jwt_secret: Arc::new(b"test-secret".to_vec()),
    };
    let login = axum::Router::new()
        .route("/api/user/login", axum::routing::post(solo_new_api_rust::routes::user::login));
    let guarded = axum::Router::new()
        .route("/api/user/self", axum::routing::get(solo_new_api_rust::routes::user::self_info))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), solo_new_api_rust::auth::admin::admin_auth));
    let app = login.merge(guarded).with_state(state);

    let res = app.clone().oneshot(Request::builder().method("POST").uri("/api/user/login")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"username":"alley","password":"secret123"}"#)).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let token = v["data"]["access_token"].as_str().unwrap().to_string();
    assert!(v["data"]["session"]["sid"].as_str().is_some());

    let res = app.oneshot(Request::builder().uri("/api/user/self")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
}
