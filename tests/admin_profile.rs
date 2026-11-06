use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, Request};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::HashMap;
use tower::ServiceExt;

use solo_new_api_rust::{auth::session, config::Config, model::UserRow, routes, state::{AppState, Caches}};

const OLD_PASSWORD: &str = "oldsecret1";
const NEW_PASSWORD: &str = "newsecret2";

struct Harness {
    app: Router,
    pool: SqlitePool,
}

impl Harness {
    async fn request(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> (StatusCode, Value, HeaderMap) {
        let builder = Request::builder().method(method).uri(uri);
        let builder = match token {
            Some(t) => builder.header("authorization", format!("Bearer {t}")),
            None => builder,
        };
        let (builder, payload) = match body {
            Some(v) => (builder.header("content-type", "application/json"), Body::from(v.to_string())),
            None => (builder, Body::empty()),
        };
        let response = self.app.clone().oneshot(builder.body(payload).unwrap()).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value, headers)
    }

    async fn login(&self) -> String {
        self.login_bundle().await["access_token"].as_str().unwrap().to_string()
    }

    async fn login_bundle(&self) -> Value {
        let (_, body, _) = self
            .request("POST", "/api/user/login", Some(json!({"username": "alley", "password": OLD_PASSWORD})), None)
            .await;
        assert_eq!(body["success"], true);
        body["data"].clone()
    }
}

async fn build(hash: String, setting: Option<&str>) -> Harness {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT NOT NULL, display_name TEXT, \
         role INTEGER, status INTEGER, email TEXT, `group` TEXT, setting TEXT, created_at INTEGER, \
         auth_version INTEGER NOT NULL DEFAULT 1, deleted_at TEXT, quota INTEGER DEFAULT 0, used_quota INTEGER DEFAULT 0, \
         request_count INTEGER DEFAULT 0, aff_code TEXT, aff_count INTEGER DEFAULT 0, aff_quota INTEGER DEFAULT 0, remark TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TABLE user_sessions (sid TEXT PRIMARY KEY, user_id INTEGER, version INTEGER, user_auth_version INTEGER, \
         status TEXT, refresh_hash TEXT, previous_refresh_hash TEXT, previous_valid_until INTEGER NOT NULL DEFAULT 0, \
         login_method TEXT, ip TEXT, user_agent TEXT, created_at INTEGER, last_active_at INTEGER, expires_at INTEGER, \
         revoked_at INTEGER DEFAULT 0, revoked_reason TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, password, display_name, role, status, email, `group`, setting, created_at, auth_version) \
         VALUES (1, 'alley', ?, 'Root', 100, 1, NULL, 'default', ?, 1700000000, 1)",
    )
    .bind(&hash)
    .bind(setting)
    .execute(&pool)
    .await
    .unwrap();

    let user = UserRow {
        id: 1,
        username: "alley".into(),
        password: hash,
        display_name: Some("Root".into()),
        role: 100,
        status: 1,
        group: "default".into(),
        auth_version: 1,
        email: None,
    };
    let mut caches = Caches {
        users: HashMap::new(),
        tokens: HashMap::new(),
        channels: HashMap::new(),
        abilities: HashMap::new(),
        options: HashMap::new(),
    };
    caches.users.insert(1, user);
    let config = Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(config, pool.clone(), caches, b"test-secret".to_vec(), None);

    // 与 main.rs 一致：self 路由在前，管理端 /api/user/{id} 在后。
    let self_route = Router::new()
        .route(
            "/api/user/self",
            axum::routing::get(routes::user::self_info).put(routes::user::update_self),
        )
        .route("/api/user/setting", axum::routing::put(routes::user::update_setting))
        .route("/api/user/sessions", axum::routing::get(routes::user::list_sessions))
        .route(
            "/api/user/sessions/revoke-others",
            axum::routing::post(routes::user::revoke_other_sessions),
        )
        .route(
            "/api/user/sessions/{sid}",
            axum::routing::delete(routes::user::revoke_session),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::auth::admin::admin_auth,
        ));
    let app = Router::new()
        .route("/api/user/login", axum::routing::post(routes::user::login))
        .merge(routes::admin::user::router())
        .merge(self_route)
        .with_state(state);
    Harness { app, pool }
}

async fn harness() -> Harness {
    build(session::hash_password(OLD_PASSWORD).unwrap(), None).await
}

async fn insert_session(pool: &SqlitePool, sid: &str, status: &str, auth_version: i64, last_active: i64) {
    sqlx::query(
        "INSERT INTO user_sessions (sid, user_id, version, user_auth_version, status, refresh_hash, login_method, ip, user_agent, created_at, last_active_at, expires_at) \
         VALUES (?, 1, 1, ?, ?, 'hash', 'password', '10.0.0.1', 'UA', 1700000000, ?, 9999999999)",
    )
    .bind(sid)
    .bind(auth_version)
    .bind(status)
    .bind(last_active)
    .execute(pool)
    .await
    .unwrap();
}

async fn setting_column(pool: &SqlitePool) -> String {
    sqlx::query_scalar("SELECT setting FROM users WHERE id=1")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn password_columns(pool: &SqlitePool) -> (String, i64) {
    sqlx::query_as("SELECT password, auth_version FROM users WHERE id=1")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 个人资料页第一步：GET /api/user/self 要带上 Go 写的 setting 与密码状态。
#[tokio::test]
async fn self_info_exposes_setting_and_password_state() {
    let h = build(session::hash_password(OLD_PASSWORD).unwrap(), Some(r#"{"language":"zh"}"#)).await;
    let token = h.login().await;
    let (status, body, _) = h.request("GET", "/api/user/self", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["username"], "alley");
    assert_eq!(body["data"]["setting"], r#"{"language":"zh"}"#);
    assert_eq!(body["data"]["created_time"], 1700000000);
    assert_eq!(body["data"]["has_password"], true);
}

#[tokio::test]
async fn update_display_name_persists_and_updates_cache() {
    let h = harness().await;
    let token = h.login().await;
    let (status, body, _) = h
        .request("PUT", "/api/user/self", Some(json!({"display_name": "  主账号  "})), Some(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["success"], true);

    let stored: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id=1")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(stored, "主账号");
    let (_, profile, _) = h.request("GET", "/api/user/self", None, Some(&token)).await;
    assert_eq!(profile["data"]["display_name"], "主账号");
}

#[tokio::test]
async fn update_display_name_rejects_overlong_value() {
    let h = harness().await;
    let token = h.login().await;
    let long = "名".repeat(21);
    let (_, body, _) = h
        .request("PUT", "/api/user/self", Some(json!({"display_name": long})), Some(&token))
        .await;
    assert_eq!(body["success"], false);
    let stored: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id=1")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(stored, "Root");
}

/// setting 由 Go 与本进程共用：合并写入必须保留本进程不认识的键。
#[tokio::test]
async fn language_is_merged_into_setting_preserving_unknown_keys() {
    let h = build(
        session::hash_password(OLD_PASSWORD).unwrap(),
        Some(r#"{"quota_warning_threshold":500,"language":"en"}"#),
    )
    .await;
    let token = h.login().await;
    let (_, body, _) = h
        .request("PUT", "/api/user/self", Some(json!({"language": "zh-CN"})), Some(&token))
        .await;
    assert_eq!(body["success"], true);
    let merged: Value = serde_json::from_str(&setting_column(&h.pool).await).unwrap();
    assert_eq!(merged["language"], "zh-CN");
    assert_eq!(merged["quota_warning_threshold"], 500);
}

#[tokio::test]
async fn update_setting_merges_patch_into_existing_json() {
    let h = build(session::hash_password(OLD_PASSWORD).unwrap(), Some(r#"{"language":"zh"}"#)).await;
    let token = h.login().await;
    let (_, body, _) = h
        .request("PUT", "/api/user/setting", Some(json!({"record_ip_log": false, "sidebar_modules": "home"})), Some(&token))
        .await;
    assert_eq!(body["success"], true);
    let merged: Value = serde_json::from_str(&setting_column(&h.pool).await).unwrap();
    assert_eq!(merged["language"], "zh");
    assert_eq!(merged["record_ip_log"], false);
    assert_eq!(merged["sidebar_modules"], "home");
}

#[tokio::test]
async fn password_change_requires_correct_original_password() {
    let h = harness().await;
    let token = h.login().await;
    let before = password_columns(&h.pool).await;

    let (_, body, _) = h
        .request(
            "PUT",
            "/api/user/self",
            Some(json!({"password": NEW_PASSWORD, "original_password": "wrong-password"})),
            Some(&token),
        )
        .await;
    assert_eq!(body["success"], false);
    assert!(body["message"].as_str().unwrap().contains("Original password"));
    assert_eq!(password_columns(&h.pool).await, before);

    let (_, body, _) = h
        .request(
            "PUT",
            "/api/user/self",
            Some(json!({"password": "short", "original_password": OLD_PASSWORD})),
            Some(&token),
        )
        .await;
    assert_eq!(body["success"], false);
    assert_eq!(password_columns(&h.pool).await, before);
    let (_, still_works, _) = h.request("GET", "/api/user/self", None, Some(&token)).await;
    assert_eq!(still_works["success"], true);
}

#[tokio::test]
async fn password_change_rotates_current_session_and_kills_others() {
    let h = harness().await;
    let session_before = h.login_bundle().await;
    let token = session_before["access_token"].as_str().unwrap().to_string();
    let login_sid = session_before["session"]["sid"].as_str().unwrap().to_string();
    insert_session(&h.pool, "other-sid", "active", 1, 1700000100).await;
    let other_token = {
        let claims = session::Claims {
            sub: 1,
            sid: "other-sid".into(),
            ver: 1,
            uver: 1,
            iat: session::now(),
            exp: session::now() + 600,
        };
        session::issue_access(&claims, b"test-secret").unwrap()
    };

    let (status, body, _) = h
        .request(
            "PUT",
            "/api/user/self",
            Some(json!({"password": NEW_PASSWORD, "original_password": OLD_PASSWORD})),
            Some(&token),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["has_password"], true);
    let rotated = body["data"]["access_token"].as_str().unwrap().to_string();
    assert_ne!(rotated, token);
    // 前端 applyAuthRotation 要求轮换回来的还是同一个会话。
    assert_eq!(body["data"]["session"]["sid"], login_sid);
    assert_eq!(body["data"]["session"]["current"], true);

    let (hash, auth_version) = password_columns(&h.pool).await;
    assert_eq!(auth_version, 2);
    assert!(session::verify_password(&hash, NEW_PASSWORD));
    assert!(!session::verify_password(&hash, OLD_PASSWORD));

    let (_, ok, _) = h.request("GET", "/api/user/self", None, Some(&rotated)).await;
    assert_eq!(ok["success"], true);
    assert_eq!(h.request("GET", "/api/user/self", None, Some(&token)).await.1["success"], false);
    assert_eq!(h.request("GET", "/api/user/self", None, Some(&other_token)).await.0, StatusCode::UNAUTHORIZED);

    // 旧密码登录失败，新密码成功。
    let (_, stale, _) = h
        .request("POST", "/api/user/login", Some(json!({"username": "alley", "password": OLD_PASSWORD})), None)
        .await;
    assert_eq!(stale["success"], false);
    let (_, fresh, _) = h
        .request("POST", "/api/user/login", Some(json!({"username": "alley", "password": NEW_PASSWORD})), None)
        .await;
    assert_eq!(fresh["success"], true);
    // 再次登录会开出新会话，而不是复用改密前那个。
    assert_ne!(fresh["data"]["session"]["sid"], json!(login_sid));
}

/// Go 回滚后只认这一种串：参数、salt/key 长度与总长度都被断言。
#[test]
fn password_hash_matches_the_format_go_accepts() {
    let encoded = session::hash_password("correct horse").unwrap();
    let parts: Vec<&str> = encoded.split('$').collect();
    assert_eq!(parts.len(), 6);
    assert_eq!(parts[0], "");
    assert_eq!(parts[1], "argon2id");
    assert_eq!(parts[2], "v=19");
    assert_eq!(parts[3], "m=19456,t=2,p=1");
    assert!(encoded.len() <= 256);

    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
    let salt = engine.decode(parts[4]).unwrap();
    let key = engine.decode(parts[5]).unwrap();
    assert_eq!(salt.len(), 16);
    assert_eq!(key.len(), 32);

    // 串里写的参数必须就是算出这段 key 用的参数。
    use argon2::{Argon2, Params, Version};
    let params = Params::new(19456, 2, 1, Some(32)).unwrap();
    let mut recomputed = [0_u8; 32];
    Argon2::new(argon2::Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(b"correct horse", &salt, &mut recomputed)
        .unwrap();
    assert_eq!(recomputed.to_vec(), key);
    assert!(session::verify_password(&encoded, "correct horse"));
    assert!(!session::verify_password(&encoded, "Correct Horse"));
}

/// 列表口径与 Go 一致：只留 active、未过期、且属于当前 auth_version 的会话。
#[tokio::test]
async fn sessions_list_excludes_revoked_expired_and_stale_version() {
    let h = harness().await;
    let token = h.login().await;
    let sid = sqlx::query_scalar::<_, String>("SELECT sid FROM user_sessions WHERE user_id=1")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    insert_session(&h.pool, "revoked", "revoked", 1, 1700000500).await;
    insert_session(&h.pool, "expired", "active", 1, 1700000400).await;
    sqlx::query("UPDATE user_sessions SET expires_at=1 WHERE sid='expired'")
        .execute(&h.pool)
        .await
        .unwrap();
    insert_session(&h.pool, "stale-version", "active", 9, 1700000300).await;
    insert_session(&h.pool, "kept", "active", 1, 1700000200).await;

    let (_, body, _) = h.request("GET", "/api/user/sessions", None, Some(&token)).await;
    let mut sids: Vec<&str> = body["data"].as_array().unwrap().iter().map(|s| s["sid"].as_str().unwrap()).collect();
    sids.sort_unstable();
    let mut expected = vec![sid.as_str(), "kept"];
    expected.sort_unstable();
    assert_eq!(sids, expected);
}

/// 当前会话排首位由 Go 保证；其余按最后活跃时间倒序。
#[tokio::test]
async fn current_session_leads_the_list() {
    let h = harness().await;
    let token = h.login().await;
    let sid = sqlx::query_scalar::<_, String>("SELECT sid FROM user_sessions WHERE user_id=1")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    insert_session(&h.pool, "newer-other", "active", 1, 1700000999).await;
    insert_session(&h.pool, "older-other", "active", 1, 1700000000).await;

    let (_, body, _) = h.request("GET", "/api/user/sessions", None, Some(&token)).await;
    let list = body["data"].as_array().unwrap();
    assert_eq!(list[0]["sid"], sid);
    assert_eq!(list[0]["current"], true);
    assert_eq!(list[1]["sid"], "newer-other");
    assert_eq!(list[2]["sid"], "older-other");
    assert_eq!(list[1]["current"], false);
}

#[tokio::test]
async fn revoke_session_marks_row_and_clears_cookie_for_current() {
    let h = harness().await;
    let token = h.login().await;
    let sid = sqlx::query_scalar::<_, String>("SELECT sid FROM user_sessions WHERE user_id=1")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    insert_session(&h.pool, "other-sid", "active", 1, 1700000000).await;

    let (status, body, headers) = h
        .request("DELETE", "/api/user/sessions/other-sid", None, Some(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["revoked_sid"], "other-sid");
    assert_eq!(body["data"]["current"], false);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM user_sessions WHERE sid='other-sid'")
            .fetch_one(&h.pool)
            .await
            .unwrap(),
        "revoked"
    );
    assert!(!headers.contains_key("set-cookie"));

    let (_, body, headers) = h.request("DELETE", &format!("/api/user/sessions/{sid}"), None, Some(&token)).await;
    assert_eq!(body["data"]["current"], true);
    assert!(headers["set-cookie"].to_str().unwrap().contains("new_api_refresh=;"));
}

#[tokio::test]
async fn revoke_unknown_or_foreign_session_returns_not_found() {
    let h = harness().await;
    let token = h.login().await;
    insert_session(&h.pool, "someone-elses", "active", 1, 1700000000).await;
    sqlx::query("UPDATE user_sessions SET user_id=2 WHERE sid='someone-elses'")
        .execute(&h.pool)
        .await
        .unwrap();

    let (status, body, _) = h.request("DELETE", "/api/user/sessions/missing", None, Some(&token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "AUTH_SESSION_NOT_FOUND");

    let (status, body, _) = h.request("DELETE", "/api/user/sessions/someone-elses", None, Some(&token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "AUTH_SESSION_NOT_FOUND");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM user_sessions WHERE sid='someone-elses'")
            .fetch_one(&h.pool)
            .await
            .unwrap(),
        "active"
    );
}

#[tokio::test]
async fn revoke_others_skips_the_current_session() {
    let h = harness().await;
    let token = h.login().await;
    insert_session(&h.pool, "other-a", "active", 1, 1700000000).await;
    insert_session(&h.pool, "other-b", "active", 1, 1700000000).await;
    insert_session(&h.pool, "already-gone", "revoked", 1, 1700000000).await;

    let (_, body, _) = h.request("POST", "/api/user/sessions/revoke-others", None, Some(&token)).await;
    assert_eq!(body["data"]["revoked_count"], 2);
    let active: Vec<String> = sqlx::query_scalar("SELECT sid FROM user_sessions WHERE status='active'")
        .fetch_all(&h.pool)
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    // 剩下的那一个就是当前会话：同一个 token 仍然可用。
    let (_, profile, _) = h.request("GET", "/api/user/self", None, Some(&token)).await;
    assert_eq!(profile["success"], true);
}

/// 审计里 /api/user/sessions 被 /api/user/{id} 抢走并返回 400；静态段必须优先。
#[tokio::test]
async fn profile_routes_are_not_shadowed_by_user_id_route() {
    let h = harness().await;
    let token = h.login().await;

    let (status, body, _) = h.request("GET", "/api/user/sessions", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["data"].is_array());

    let (status, body, _) = h.request("GET", "/api/user/1", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["username"], "alley");

    let (status, _, _) = h
        .request("PUT", "/api/user/setting", Some(json!({"language": "zh"})), Some(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(setting_column(&h.pool).await.contains("language"));
}
