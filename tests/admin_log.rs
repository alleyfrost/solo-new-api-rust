use axum::body::Body;
use axum::http::Request;
use axum::Router;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::state::{AppState, Caches};

async fn setup() -> (Router, SqlitePool, i64) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE perf_metrics (id INTEGER PRIMARY KEY AUTOINCREMENT,model_name TEXT,`group` TEXT,bucket_ts INTEGER,request_count INTEGER DEFAULT 0,success_count INTEGER DEFAULT 0,total_latency_ms INTEGER DEFAULT 0,ttft_sum_ms INTEGER DEFAULT 0,ttft_count INTEGER DEFAULT 0,output_tokens INTEGER DEFAULT 0,generation_ms INTEGER DEFAULT 0)").execute(&pool).await.unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for (model, tokens, ty) in [("m1", 10, 2), ("m1", 20, 2), ("m2", 30, 5)] {
        sqlx::query("INSERT INTO logs (user_id,created_at,type,content,username,token_name,model_name,quota,prompt_tokens,completion_tokens,use_time,is_stream,channel_id,channel_name,token_id,`group`,ip,request_id,upstream_request_id,other) VALUES (1,?,?,NULL,'root','t',?,0,?,?,1,1,3,'c',2,'default','127.0.0.1','','','{}')")
            .bind(now - 10)
            .bind(ty)
            .bind(model)
            .bind(tokens)
            .bind(tokens / 2)
            .execute(&pool)
            .await
            .unwrap();
    }

    sqlx::query("UPDATE logs SET other='{\"cache_tokens\":7,\"cache_creation_tokens\":3}' WHERE model_name='m1' AND prompt_tokens=10")
        .execute(&pool)
        .await
        .unwrap();
    let bucket = now / 3600 * 3600;
    for (model, requests, successes, latency, output, generation) in [
        ("m1", 3, 2, 3000, 30, 3000),
        ("m2", 1, 0, 1000, 0, 1000),
    ] {
        sqlx::query("INSERT INTO perf_metrics (model_name,`group`,bucket_ts,request_count,success_count,total_latency_ms,ttft_sum_ms,ttft_count,output_tokens,generation_ms) VALUES (?, 'default', ?, ?, ?, ?, 0, 0, ?, ?)")
            .bind(model)
            .bind(bucket)
            .bind(requests)
            .bind(successes)
            .bind(latency)
            .bind(output)
            .bind(generation)
            .execute(&pool)
            .await
            .unwrap();
    }

    let caches = Caches::load(&pool).await.unwrap();
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(cfg, pool.clone(), caches, b"test-secret".to_vec(), None);
    let app = solo_new_api_rust::routes::admin::log::router()
        .merge(solo_new_api_rust::routes::admin::data::router())
        .merge(solo_new_api_rust::routes::admin::misc::router())
        .with_state(state);
    (app, pool, now)
}

async fn get(app: &Router, uri: &str) -> (u16, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

#[tokio::test]
async fn log_list_filters_and_stats() {
    let (app, _, now) = setup().await;

    let (status, list) = get(&app, "/api/log?p=1&page_size=10&type=2").await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["data"]["total"], 2);
    assert_eq!(list["data"]["items"][0]["channel"], 3);
    assert_eq!(list["data"]["items"][0]["group"], "default");
    assert_eq!(list["data"]["items"][0]["is_stream"], true);

    let (_, filtered) = get(&app, "/api/log?p=1&page_size=10&model_name=m1").await;
    assert_eq!(filtered["data"]["total"], 2);
    let (_, filtered) = get(&app, "/api/log?p=1&page_size=10&model_name=m2").await;
    assert_eq!(filtered["data"]["total"], 1);

    let (_, stat) = get(&app, "/api/log/stat?type=2").await;
    assert_eq!(stat["data"]["quota"], 0);
    assert_eq!(stat["data"]["rpm"], 2);
    assert_eq!(stat["data"]["tpm"], 45);

    let (_status, data) = get(&app, &format!("/api/data?start_timestamp={}&end_timestamp={}", now - 3600, now + 1)).await;
    assert_eq!(data["data"].as_array().unwrap().len(), 1);
    let m1 = data["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["model_name"] == "m1")
        .unwrap();
    assert_eq!(m1["token_used"], 45);
    assert_eq!(m1["count"], 2);
    assert_eq!(m1["cache_read_tokens"], 7);
    assert_eq!(m1["cache_creation_tokens"], 3);

    let (_, users) = get(&app, &format!("/api/data/users?start_timestamp={}&end_timestamp={}", now - 3600, now + 1)).await;
    assert_eq!(users["data"][0]["username"], "root");

    let (_, flow) = get(&app, &format!("/api/data/flow?start_timestamp={}&end_timestamp={}", now - 3600, now + 1)).await;
    assert_eq!(flow["data"][0]["channel_name"], "c");
    assert_eq!(flow["data"][0]["token_name"], "t");

    let (_, uptime) = get(&app, "/api/uptime/status").await;
    assert_eq!(uptime["data"].as_array().unwrap().len(), 0);

    let (status, perf) = get(&app, "/api/perf-metrics/summary?hours=1").await;
    assert_eq!(status, 200, "{perf}");
    let models = perf["data"]["models"].as_array().unwrap();
    assert_eq!(models[0]["model_name"], "m1");
    assert_eq!(models[0]["request_count"], 3);
    assert_eq!(models[0]["success_rate"], 66.67);
    assert_eq!(models[0]["avg_latency_ms"], 1000.0);
    assert_eq!(models[0]["avg_tps"], 10.0);
    assert_eq!(perf["data"]["summary"]["success_rate"], 50.0);
    assert_eq!(perf["data"]["summary"]["avg_latency_ms"], 1000.0);
    assert_eq!(perf["data"]["summary"]["avg_tps"], 7.5);
}
