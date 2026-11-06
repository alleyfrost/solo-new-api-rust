use axum::body::Body;
use axum::extract::Request;
use axum::http::header::CONTENT_TYPE;
use axum::http::Request as HttpRequest;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::auth::admin::AdminContext;
use solo_new_api_rust::config::Config;
use solo_new_api_rust::routes::admin::log::MAX_EXPORT_DAYS;
use solo_new_api_rust::state::{AppState, Caches};

const DAY_SECS: i64 = 24 * 60 * 60;

async fn inject_session(
    mut req: Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, solo_new_api_rust::error::ApiError> {
    req.extensions_mut().insert(AdminContext {
        user_id: 1,
        sid: "test-sid".into(),
    });
    Ok(next.run(req).await)
}

/// `rows` 条当前用户 + 3 条其他用户的日志；行数超过导出分页大小以覆盖翻页。
async fn setup(rows: i64) -> (Router, SqlitePool, i64) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    for table in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, setting TEXT, created_at INTEGER, deleted_at TEXT)",
        "CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)",
        "CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)",
        "CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)",
        "CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)",
    ] {
        sqlx::query(table).execute(&pool).await.unwrap();
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    // 单条语句生成全部行，避免上千次往返。
    sqlx::query(
        "WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM s WHERE i < ?) \
         INSERT INTO logs (user_id,created_at,type,content,username,token_name,model_name,quota,prompt_tokens,completion_tokens,use_time,is_stream,channel_id,channel_name,token_id,`group`,ip,request_id,upstream_request_id,other) \
         SELECT 1, ?, 2, NULL, 'root', 't', 'm1', 0, 1, 2, 1, 1, 3, 'c', 2, 'default', '127.0.0.1', '', '', '{}' FROM s",
    )
    .bind(rows)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    for offset in [10, 20, 30] {
        sqlx::query("INSERT INTO logs (user_id,created_at,type,content,username,token_name,model_name,quota,prompt_tokens,completion_tokens,use_time,is_stream,channel_id,channel_name,token_id,`group`,ip,request_id,upstream_request_id,other) VALUES (2,?,2,NULL,'other','t','m1',0,1,2,1,1,3,'c',2,'default','127.0.0.1','','','{}')")
            .bind(now - offset)
            .execute(&pool)
            .await
            .unwrap();
    }

    let caches = Caches::load(&pool).await.unwrap();
    let config = Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(config, pool.clone(), caches, b"test-secret".to_vec(), None);
    let app = solo_new_api_rust::routes::admin::log::router()
        .route_layer(axum::middleware::from_fn(inject_session))
        .with_state(state);
    (app, pool, now)
}

/// NDJSON 流：逐行解析日志，最后一行是结果对象。
async fn get_stream(app: &Router, uri: &str) -> (u16, String, Vec<Value>) {
    let res = app
        .clone()
        .oneshot(HttpRequest::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let content_type = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let lines = String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each ndjson line is a JSON value"))
        .collect();
    (status, content_type, lines)
}

#[tokio::test]
async fn export_streams_every_row_then_count() {
    let rows = 2_500;
    let (app, _, _) = setup(rows).await;
    let (status, content_type, lines) = get_stream(&app, "/api/log/export?type=2").await;
    assert_eq!(status, 200);
    assert_eq!(content_type, "application/x-ndjson; charset=utf-8");

    // 管理端导出不限用户：本用户 rows 条 + 其他用户 3 条。
    let total = rows + 3;
    let last = lines.last().unwrap();
    assert_eq!(last["success"], true);
    assert_eq!(last["data"]["count"], total);
    assert_eq!(lines.len() as i64, total + 1);

    let logs = &lines[..lines.len() - 1];
    assert_eq!(logs[0]["type"], 2);
    // id 倒序：其他用户后插入，id 更大排在最前；当前用户最早的行落在末尾。
    assert_eq!(logs[0]["username"], "other");
    assert_eq!(logs[logs.len() - 1]["username"], "root");
    let ids: Vec<i64> = logs.iter().map(|v| v["id"].as_i64().unwrap()).collect();
    assert_eq!(logs.iter().filter(|v| v["user_id"] == 2).count(), 3);
    assert!(
        ids.windows(2).all(|w| w[0] > w[1]),
        "导出必须按 id 倒序，且翻页不能重复或漏行"
    );
    assert_eq!(ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len());
}

#[tokio::test]
async fn self_export_only_streams_the_callers_rows() {
    let (app, _, _) = setup(5).await;
    let (_, _, lines) = get_stream(&app, "/api/log/self/export").await;
    let last = lines.last().unwrap();
    assert_eq!(last["data"]["count"], 5);
    assert!(
        lines[..lines.len() - 1].iter().all(|v| v["user_id"] == 1),
        "自助导出不能带出别人的日志"
    );
}

#[tokio::test]
async fn export_rejects_overlong_range() {
    let (app, _, now) = setup(5).await;
    let uri = format!(
        "/api/log/export?start_timestamp={}&end_timestamp={}",
        now - (MAX_EXPORT_DAYS + 30) * DAY_SECS,
        now
    );
    let (status, _, lines) = get_stream(&app, &uri).await;
    // 业务错误走 Go 口径：HTTP 200 + success:false。
    assert_eq!(status, 200);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["success"], false);
    assert!(
        lines[0]["message"]
            .as_str()
            .unwrap()
            .contains("The time range cannot exceed 365 days"),
        "{}",
        lines[0]
    );
}

#[tokio::test]
async fn export_is_serial_per_user() {
    let (app, _, _) = setup(5).await;
    // 第一个响应只取头部、不消费 body，导出槽位应保持占用。
    let first = app
        .clone()
        .oneshot(
            HttpRequest::get("/api/log/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, _, lines) = get_stream(&app, "/api/log/export").await;
    // 并发拒绝同样是 Go 口径：HTTP 200 + success:false。
    assert_eq!(status, 200);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["success"], false);
    assert!(
        lines[0]["message"]
            .as_str()
            .unwrap()
            .contains("Another log export")
    );

    // 释放后同一用户可以再次导出。
    drop(first);
    let (_, _, lines) = get_stream(&app, "/api/log/export").await;
    assert_eq!(lines.last().unwrap()["success"], true);
}
