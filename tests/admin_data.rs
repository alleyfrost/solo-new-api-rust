use axum::Router;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::state::{AppState, Caches};

async fn pool_with_tables() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE quota_data (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,username TEXT DEFAULT '',model_name TEXT DEFAULT '',created_at INTEGER,use_group TEXT DEFAULT '',token_id INTEGER DEFAULT 0,channel_id INTEGER DEFAULT 0,node_name TEXT DEFAULT '',token_used INTEGER DEFAULT 0,count INTEGER DEFAULT 0,quota INTEGER DEFAULT 0,cache_read_tokens INTEGER DEFAULT 0,cache_creation_tokens INTEGER DEFAULT 0)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE perf_metrics (id INTEGER PRIMARY KEY AUTOINCREMENT,model_name TEXT,`group` TEXT,bucket_ts INTEGER,request_count INTEGER DEFAULT 0,success_count INTEGER DEFAULT 0,total_latency_ms INTEGER DEFAULT 0,ttft_sum_ms INTEGER DEFAULT 0,ttft_count INTEGER DEFAULT 0,output_tokens INTEGER DEFAULT 0,generation_ms INTEGER DEFAULT 0)").execute(&pool).await.unwrap();
    pool
}

async fn app(pool: SqlitePool) -> Router {
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
    solo_new_api_rust::routes::admin::data::router().with_state(state)
}

async fn get(app: &Router, uri: &str) -> (u16, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn default_time_hour_buckets_quota_data_by_hour() {
    let pool = pool_with_tables().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let bucket_now = now / 3600 * 3600;
    let bucket_prev = bucket_now - 3600;
    for (model, bucket, tokens) in [
        ("m1", bucket_prev, 10),
        ("m1", bucket_now, 20),
        ("m2", bucket_now, 5),
    ] {
        sqlx::query("INSERT INTO quota_data (user_id,username,model_name,created_at,use_group,token_id,channel_id,node_name,token_used,count,quota,cache_read_tokens,cache_creation_tokens) VALUES (1,'root',?,?,'default',0,0,'',?,1,0,0,0)")
            .bind(model)
            .bind(bucket)
            .bind(tokens)
            .execute(&pool)
            .await
            .unwrap();
    }
    let app = app(pool).await;
    let url = format!(
        "/api/data?start_timestamp={}&end_timestamp={}",
        bucket_prev - 60,
        bucket_now + 60
    );

    // 缺省按天聚合（现状）：m1 两个相邻小时桶可能合并为一天或跨 UTC 日界
    // 拆成两天，这里按模型合计校验总额。
    let (_, day) = get(&app, &url).await;
    let day_rows = day["data"].as_array().unwrap();
    let sum_by_model = |model: &str| {
        day_rows
            .iter()
            .filter(|v| v["model_name"] == model)
            .map(|v| v["token_used"].as_i64().unwrap())
            .sum::<i64>()
    };
    assert_eq!(sum_by_model("m1"), 30, "{day}");
    assert_eq!(sum_by_model("m2"), 5, "{day}");
    for row in day_rows {
        assert_eq!(row["created_at"].as_i64().unwrap() % 86400, 0, "{row}");
    }

    // hour 粒度：三个小时桶逐小时可见，created_at 按小时对齐。
    let (_, hour) = get(&app, &format!("{url}&default_time=hour")).await;
    let rows = hour["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{hour}");
    for row in rows {
        let bucket = row["created_at"].as_i64().unwrap();
        assert_eq!(bucket % 3600, 0, "{row}");
        assert!(bucket == bucket_now || bucket == bucket_prev, "{row}");
    }
    let m1_now = rows
        .iter()
        .find(|v| v["model_name"] == "m1" && v["created_at"] == bucket_now)
        .unwrap();
    assert_eq!(m1_now["token_used"], 20);
    let m2_now = rows.iter().find(|v| v["model_name"] == "m2").unwrap();
    assert_eq!(m2_now["created_at"].as_i64().unwrap(), bucket_now);
    assert_eq!(m2_now["token_used"], 5);

    // 非法 default_time 降级为按天分组，不报错。
    let (status, bogus) = get(&app, &format!("{url}&default_time=bogus")).await;
    assert_eq!(status, 200, "{bogus}");
    let bogus_rows = bogus["data"].as_array().unwrap();
    for row in bogus_rows {
        assert_eq!(row["created_at"].as_i64().unwrap() % 86400, 0, "{row}");
    }
    let m1_total = bogus_rows
        .iter()
        .filter(|v| v["model_name"] == "m1")
        .map(|v| v["token_used"].as_i64().unwrap())
        .sum::<i64>();
    assert_eq!(m1_total, 30, "{bogus}");
}

#[tokio::test]
async fn default_time_hour_buckets_logs_fallback() {
    let pool = pool_with_tables().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let bucket_now = now / 3600 * 3600;
    let bucket_prev = bucket_now - 3600;
    for (model, bucket, tokens) in [("m1", bucket_prev, 10), ("m1", bucket_now, 20)] {
        sqlx::query("INSERT INTO logs (user_id,created_at,type,username,model_name,quota,prompt_tokens,completion_tokens,`group`,other) VALUES (1,?,2,'root',?,0,?,0,'default','{}')")
            .bind(bucket)
            .bind(model)
            .bind(tokens)
            .execute(&pool)
            .await
            .unwrap();
    }
    let app = app(pool).await;
    let url = format!(
        "/api/data?start_timestamp={}&end_timestamp={}",
        bucket_prev - 60,
        bucket_now + 60
    );

    let (_, day) = get(&app, &url).await;
    let day_rows = day["data"].as_array().unwrap();
    let total_tokens = day_rows
        .iter()
        .map(|v| v["token_used"].as_i64().unwrap())
        .sum::<i64>();
    assert_eq!(total_tokens, 30, "{day}");
    for row in day_rows {
        assert_eq!(row["created_at"].as_i64().unwrap() % 86400, 0, "{row}");
    }

    let (_, hour) = get(&app, &format!("{url}&default_time=hour")).await;
    let rows = hour["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{hour}");
    for row in rows {
        assert_eq!(row["created_at"].as_i64().unwrap() % 3600, 0, "{row}");
    }
    let first = rows
        .iter()
        .find(|v| v["created_at"] == bucket_prev)
        .unwrap();
    assert_eq!(first["token_used"], 10);
}
