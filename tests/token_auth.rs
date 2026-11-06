use std::collections::HashMap;
use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use http_body_util::BodyExt;
use tower::ServiceExt;

fn caches() -> solo_new_api_rust::state::Caches {
    let mut caches = solo_new_api_rust::state::Caches {
        users: HashMap::new(), tokens: HashMap::new(), channels: HashMap::new(),
        abilities: HashMap::new(), options: HashMap::new(),
    };
    caches.users.insert(1, solo_new_api_rust::model::UserRow {
        id: 1, username: "root".into(), password: "x".into(), display_name: None,
        role: 100, status: 1, group: "default".into(), auth_version: 1, email: None,
    });
    caches.tokens.insert("abc123".into(), solo_new_api_rust::model::TokenRow {
        id: 2, user_id: 1, key: "abc123".into(), status: 1, name: "t".into(),
        expired_time: -1, remain_quota: 0, unlimited_quota: true,
        model_limits_enabled: false, model_limits: String::new(), allow_ips: None,
        group: "vip".into(), auto_groups: None, cross_group_retry: false,
    });
    caches
}

fn state_with(caches: solo_new_api_rust::state::Caches) -> solo_new_api_rust::state::AppState {
    let cfg = solo_new_api_rust::config::Config {
        port: 0, db_path: ":memory:".into(), web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(), trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    solo_new_api_rust::state::AppState::new(
        cfg,
        sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap(),
        caches,
        b"test-secret".to_vec(),
        None,
    )
}

fn state() -> solo_new_api_rust::state::AppState {
    state_with(caches())
}

async fn whoami(ctx: axum::extract::Extension<solo_new_api_rust::auth::token::TokenContext>) -> String {
    ctx.0.token_group
}

fn app_with(state: solo_new_api_rust::state::AppState) -> axum::Router {
    axum::Router::new()
        .route("/protected", get(whoami))
        .layer(axum::middleware::from_fn_with_state(
            state,
            solo_new_api_rust::auth::token::token_auth,
        ))
}

async fn call(builder: axum::http::request::Builder) -> axum::http::Response<Body> {
    app_with(state())
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn rejects_missing_and_accepts_sk_key() {
    let app = app_with(state());

    let res = app.clone().oneshot(Request::builder().uri("/protected").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 401);

    let res = app.oneshot(Request::builder().uri("/protected")
        .header("authorization", "Bearer sk-abc123")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"vip");
}

#[tokio::test]
async fn accepts_lowercase_bearer_x_api_key_and_query_key() {
    for req in [
        Request::builder().uri("/protected").header("authorization", "bearer sk-abc123"),
        Request::builder().uri("/protected").header("x-api-key", "sk-abc123"),
        Request::builder().uri("/protected").header("x-api-key", "Bearer sk-abc123"),
        Request::builder().uri("/protected?key=sk-abc123"),
        Request::builder().uri("/protected?foo=1&key=abc123"),
    ] {
        let res = call(req).await;
        assert_eq!(res.status(), 200);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"vip");
    }
}

#[tokio::test]
async fn takes_first_segment_after_dash_and_sk_prefix() {
    let res = call(Request::builder().uri("/protected")
        .header("authorization", "Bearer sk-abc123-extra-suffix")).await;
    assert_eq!(res.status(), 200);

    let res = call(Request::builder().uri("/protected")
        .header("x-api-key", "abc123-tail")).await;
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn rejects_unknown_key() {
    let res = call(Request::builder().uri("/protected")
        .header("authorization", "Bearer sk-nope")).await;
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn rejects_disabled_token_disabled_user_and_expired_token() {
    let mut disabled_token = caches();
    disabled_token.tokens.get_mut("abc123").unwrap().status = 2;
    let res = app_with(state_with(disabled_token)).oneshot(Request::builder().uri("/protected")
        .header("authorization", "Bearer sk-abc123").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 401);

    let mut disabled_user = caches();
    disabled_user.users.get_mut(&1).unwrap().status = 2;
    let res = app_with(state_with(disabled_user)).oneshot(Request::builder().uri("/protected")
        .header("authorization", "Bearer sk-abc123").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 401);

    let mut expired = caches();
    expired.tokens.get_mut("abc123").unwrap().expired_time = 1;
    let res = app_with(state_with(expired)).oneshot(Request::builder().uri("/protected")
        .header("authorization", "Bearer sk-abc123").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 401);

    let mut future = caches();
    let later = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 + 3600;
    future.tokens.get_mut("abc123").unwrap().expired_time = later;
    let res = app_with(state_with(future)).oneshot(Request::builder().uri("/protected")
        .header("authorization", "Bearer sk-abc123").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
}
