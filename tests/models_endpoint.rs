use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use http_body_util::BodyExt;
use solo_new_api_rust::auth::token::token_auth;
use solo_new_api_rust::model::{AbilityRow, ChannelRow, TokenRow, UserRow};
use solo_new_api_rust::routes::models::{list_models, retrieve_model};
use solo_new_api_rust::state::{AppState, Caches};
use tower::ServiceExt;

fn token(key: &str, group: &str, limits_enabled: bool, limits: &str) -> TokenRow {
    TokenRow {
        id: key.len() as i64, user_id: 1, key: key.into(), status: 1, name: "t".into(),
        expired_time: -1, remain_quota: 0, unlimited_quota: true,
        model_limits_enabled: limits_enabled, model_limits: limits.into(), allow_ips: None,
        group: group.into(), auto_groups: None, cross_group_retry: false,
    }
}

fn ability(group: &str, model: &str, channel_id: i64, enabled: bool) -> AbilityRow {
    AbilityRow { group: group.into(), model: model.into(), channel_id, enabled, priority: None, weight: None }
}

fn caches() -> Caches {
    let mut caches = Caches {
        users: HashMap::new(), tokens: HashMap::new(), channels: HashMap::new(),
        abilities: HashMap::new(), options: HashMap::new(),
    };
    caches.users.insert(1, UserRow {
        id: 1, username: "root".into(), password: "x".into(), display_name: None,
        role: 100, status: 1, group: "default".into(), auth_version: 1, email: None,
    });
    caches.tokens.insert("abc123".into(), token("abc123", "vip", false, ""));
    caches.tokens.insert("limit1".into(), token("limit1", "vip", true, "m2"));
    caches.channels.insert(3, ChannelRow {
        id: 3, r#type: 1, key: "sk-up".into(), status: 1, name: "c".into(),
        models: "m1,m2,m3".into(), group: "vip".into(), base_url: None,
        priority: None, weight: None, channel_info: Some("{}".into()),
    });
    let mut abilities = HashMap::new();
    abilities.insert(("vip".to_string(), "m1".to_string()), vec![ability("vip", "m1", 3, true)]);
    abilities.insert(("vip".to_string(), "m2".to_string()), vec![ability("vip", "m2", 3, true)]);
    abilities.insert(("default".to_string(), "m3".to_string()), vec![ability("default", "m3", 3, true)]);
    caches.abilities = abilities;
    caches
}

fn state_with(caches: Caches) -> AppState {
    let cfg = solo_new_api_rust::config::Config {
        port: 0, db_path: ":memory:".into(), web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(), trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    AppState {
        config: cfg,
        pool: sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap(),
        caches: Arc::new(RwLock::new(caches)),
        jwt_secret: Arc::new(b"test-secret".to_vec()),
    }
}

fn app(caches: Caches) -> axum::Router {
    let state = state_with(caches);
    axum::Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/models/{model}", get(retrieve_model))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), token_auth))
        .with_state(state)
}

async fn get_models(builder: axum::http::request::Builder) -> (u16, serde_json::Value) {
    let res = app(caches()).oneshot(builder.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn list_models_filters_by_group_and_channel_status() {
    let (status, v) = get_models(Request::builder().uri("/v1/models")
        .header("authorization", "Bearer sk-abc123")).await;
    assert_eq!(status, 200);
    assert_eq!(v["success"], true);
    assert_eq!(v["data"]["object"], "list");
    let ids: Vec<&str> = v["data"]["data"].as_array().unwrap().iter()
        .map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["m1", "m2"]);
}

#[tokio::test]
async fn anthropic_headers_return_anthropic_shape() {
    let (status, v) = get_models(Request::builder().uri("/v1/models")
        .header("anthropic-version", "2023-06-01")
        .header("x-api-key", "sk-abc123")).await;
    assert_eq!(status, 200);
    let data = v["data"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["type"], "model");
    assert_eq!(v["data"]["has_more"], false);
}

#[tokio::test]
async fn model_limits_filter_to_allowed_list() {
    let (status, v) = get_models(Request::builder().uri("/v1/models")
        .header("authorization", "Bearer sk-limit1")).await;
    assert_eq!(status, 200);
    let ids: Vec<&str> = v["data"]["data"].as_array().unwrap().iter()
        .map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["m2"]);
}

#[tokio::test]
async fn retrieve_model_returns_model_or_404() {
    let (status, v) = get_models(Request::builder().uri("/v1/models/m1")
        .header("authorization", "Bearer sk-abc123")).await;
    assert_eq!(status, 200);
    assert_eq!(v["data"]["id"], "m1");
    assert_eq!(v["data"]["object"], "model");

    let (status, _) = get_models(Request::builder().uri("/v1/models/m3")
        .header("authorization", "Bearer sk-abc123")).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn rejects_missing_token() {
    let (status, _) = get_models(Request::builder().uri("/v1/models")).await;
    assert_eq!(status, 401);
}
