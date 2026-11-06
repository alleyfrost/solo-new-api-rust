use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::Request as HttpRequest;
use axum::http::header::CONTENT_TYPE;
use axum::routing::get;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::auth::admin::AdminContext;
use solo_new_api_rust::config::Config;
use solo_new_api_rust::routes::admin::log::MAX_EXPORT_DAYS;
use solo_new_api_rust::routes::{admin, status};
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

/// 只建管理端用到的表；日志行了足够覆盖导出与清理，user 1 = 自己，user 2 = 别人。
async fn setup(log_rows: i64) -> (Router, Router, SqlitePool, i64) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    for table in [
        "CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)",
        "CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)",
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
    sqlx::query(
        "WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM s WHERE i < ?) \
         INSERT INTO logs (user_id,created_at,type,content,username,token_name,model_name,quota,prompt_tokens,completion_tokens,use_time,is_stream,channel_id,channel_name,token_id,`group`,ip,request_id,upstream_request_id,other) \
         SELECT 1, ? - (i % 10) * 86400, 2, NULL, 'root', 't', 'm1', 0, 1, 2, 1, 1, 3, 'c', 2, 'default', '127.0.0.1', '', '', '{}' FROM s",
    )
    .bind(log_rows)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

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
    let admin_app = admin::option::router()
        .merge(admin::log::router())
        .route_layer(axum::middleware::from_fn(inject_session))
        .with_state(state.clone());
    let public_app = Router::new()
        .route("/api/status", get(status::get_status))
        .route("/api/notice", get(status::get_notice))
        .with_state(state);
    (admin_app, public_app, pool, now)
}

async fn send(app: &Router, req: Request<Body>) -> (u16, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get_json(app: &Router, uri: &str) -> Value {
    let (status, body) = send(
        app,
        Request::builder().uri(uri).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, 200, "{uri} -> {body}");
    body
}

async fn put_option(app: &Router, payload: Value) -> Value {
    let (status, body) = send(
        app,
        HttpRequest::builder()
            .method("PUT")
            .uri("/api/option/")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn delete(app: &Router, uri: &str) -> Value {
    let (status, body) = send(
        app,
        HttpRequest::builder()
            .method("DELETE")
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}

/// 导出走 NDJSON 流；校验失败时服务端直接回一个 JSON 对象，同样能按行解析。
async fn get_stream(app: &Router, uri: &str) -> Vec<Value> {
    let res = app
        .clone()
        .oneshot(HttpRequest::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect()
}

fn option_value(list: &Value, key: &str) -> String {
    list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["key"] == key)
        .unwrap_or_else(|| panic!("missing option {key}"))
        .pointer("/value")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap()
}

#[tokio::test]
async fn lists_only_supported_keys_with_defaults() {
    let (app, _, _, _) = setup(1).await;
    let list = get_json(&app, "/api/option/").await;
    let rows = list["data"].as_array().unwrap();
    assert_eq!(rows.len(), 7);
    assert_eq!(option_value(&list, "SystemName"), "solo-new-api");
    assert_eq!(option_value(&list, "Logo"), "");
    assert_eq!(option_value(&list, "LogConsumeEnabled"), "true");
    assert_eq!(
        option_value(&list, "log_setting.user_export_enabled"),
        "true"
    );
    assert_eq!(
        option_value(&list, "log_setting.max_range_days"),
        MAX_EXPORT_DAYS.to_string()
    );
}

#[tokio::test]
async fn saves_supported_keys_and_refreshes_caches() {
    let (app, public, pool, _) = setup(1).await;

    assert_eq!(
        put_option(&app, json!({"key": "SystemName", "value": "  我的网关  "})).await["success"],
        true
    );
    put_option(
        &app,
        json!({"key": "Logo", "value": "https://cdn.example.com/logo.png"}),
    )
    .await;
    put_option(
        &app,
        json!({"key": "Notice", "value": "迁移中，请降低并发"}),
    )
    .await;
    put_option(&app, json!({"key": "LogConsumeEnabled", "value": false})).await;
    put_option(
        &app,
        json!({"key": "log_setting.max_range_days", "value": 30}),
    )
    .await;

    // 落库的是归一化后的字符串，Go 写的行仍原样保留。
    sqlx::query("INSERT INTO options (`key`, value) VALUES ('RetryTimes', '10')")
        .execute(&pool)
        .await
        .unwrap();
    let stored: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT `key`, value FROM options ORDER BY `key`")
            .fetch_all(&pool)
            .await
            .unwrap();
    let stored = stored
        .into_iter()
        .map(|(k, v)| (k, v.unwrap_or_default()))
        .collect::<Vec<_>>();
    assert!(stored.contains(&("SystemName".to_string(), "我的网关".to_string())));
    assert!(stored.contains(&("LogConsumeEnabled".to_string(), "false".to_string())));
    assert!(stored.contains(&("log_setting.max_range_days".to_string(), "30".to_string())));
    assert!(stored.contains(&("RetryTimes".to_string(), "10".to_string())));

    let list = get_json(&app, "/api/option/").await;
    assert_eq!(option_value(&list, "Notice"), "迁移中，请降低并发");

    // 缓存已刷新：状态端点和公告直接反映新值。
    let status = get_json(&public, "/api/status").await;
    assert_eq!(status["data"]["system_name"], "我的网关");
    assert_eq!(status["data"]["logo"], "https://cdn.example.com/logo.png");
    assert_eq!(status["data"]["log_max_range_days"], 30);
    let notice = get_json(&public, "/api/notice").await;
    assert_eq!(notice["data"], "迁移中，请降低并发");
}

#[tokio::test]
async fn rejects_unknown_keys_and_bad_values() {
    let (app, _, pool, _) = setup(1).await;

    for payload in [
        json!({"key": "RegisterEnabled", "value": "true"}),
        json!({"key": "SystemName", "value": "   "}),
        json!({"key": "SystemName", "value": "x".repeat(65)}),
        json!({"key": "Logo", "value": "javascript:alert(1)"}),
        json!({"key": "ServerAddress", "value": "ftp://example.com"}),
        json!({"key": "LogConsumeEnabled", "value": "maybe"}),
        json!({"key": "log_setting.max_range_days", "value": 0}),
        json!({"key": "log_setting.max_range_days", "value": 4000}),
        json!({"key": "Notice", "value": null}),
    ] {
        let body = put_option(&app, payload.clone()).await;
        assert_eq!(body["success"], false, "{payload} -> {body}");
        assert!(body["message"].as_str().unwrap().len() > 3, "{body}");
    }

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM options")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "被拒的写入不能落库");
}

#[tokio::test]
async fn normalizes_server_address_and_empty_logo() {
    let (app, public, _, _) = setup(1).await;
    put_option(
        &app,
        json!({"key": "ServerAddress", "value": "https://gw.example.com/"}),
    )
    .await;
    put_option(&app, json!({"key": "Logo", "value": ""})).await;

    let status = get_json(&public, "/api/status").await;
    assert_eq!(status["data"]["server_address"], "https://gw.example.com");
    assert_eq!(status["data"]["logo"], "");
}

#[tokio::test]
async fn export_honours_option_range_and_switch() {
    let (app, _, _, now) = setup(5).await;
    put_option(
        &app,
        json!({"key": "log_setting.max_range_days", "value": 2}),
    )
    .await;

    let too_wide = get_stream(
        &app,
        &format!(
            "/api/log/export?start_timestamp={}&end_timestamp={now}",
            now - 5 * DAY_SECS
        ),
    )
    .await
    .pop()
    .unwrap();
    assert_eq!(too_wide["success"], false);
    assert!(
        too_wide["message"]
            .as_str()
            .unwrap()
            .contains("cannot exceed 2 days"),
        "{too_wide}"
    );

    let within = get_stream(
        &app,
        &format!(
            "/api/log/export?start_timestamp={}&end_timestamp={now}",
            now - 86_400
        ),
    )
    .await;
    assert_eq!(within.last().unwrap()["success"], true, "{within:?}");

    put_option(
        &app,
        json!({"key": "log_setting.user_export_enabled", "value": false}),
    )
    .await;
    let refused = get_json(&app, "/api/log/self/export").await;
    assert_eq!(refused["success"], false, "{refused}");
    // 关掉个人导出不影响管理端全量导出。
    let admin_export = get_stream(&app, "/api/log/export").await;
    assert_eq!(
        admin_export.last().unwrap()["success"],
        true,
        "{admin_export:?}"
    );
}

#[tokio::test]
async fn cleanup_deletes_only_older_rows_in_bounded_batches() {
    let (app, _, pool, now) = setup(200).await;
    let cutoff = now - 5 * DAY_SECS;

    // 200 行的 created_at 落在 now-{0..9} 天：截止点 5 天前，早于它的有 80 行。
    let first = delete(&app, &format!("/api/log/cleanup?before={cutoff}&limit=50")).await;
    assert_eq!(first["data"]["deleted"], 50, "{first}");
    assert_eq!(first["data"]["has_more"], true);
    assert_eq!(first["data"]["remaining"], 30, "{first}");

    let total_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE created_at >= ?")
        .bind(cutoff)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, 120, "晚于或等于截止点的日志必须原样保留");
    // 此刻总数只包含保留的 120 条和还没删掉的 30 条。
    assert_eq!(total_before, kept + 30);

    let mut remaining = first["data"]["remaining"].as_i64().unwrap();
    while remaining > 0 {
        let next = delete(
            &app,
            &format!("/api/log/cleanup?before={cutoff}&limit=1000"),
        )
        .await;
        remaining = next["data"]["remaining"].as_i64().unwrap();
        assert_eq!(next["data"]["has_more"], remaining > 0, "{next}");
    }

    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE created_at < ?")
        .bind(cutoff)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);

    for bad in [
        "/api/log/cleanup",
        &format!("/api/log/cleanup?before={now}"),
    ] {
        let body = delete(&app, bad).await;
        assert_eq!(body["success"], false, "{bad} -> {body}");
    }
}

#[tokio::test]
async fn consume_switch_stops_writing_logs() {
    let (app, _, pool, now) = setup(1).await;
    let (tx, rx) = solo_new_api_rust::logging::channel(8);
    let caches = Caches::load(&pool).await.unwrap();
    let state = AppState::new(
        Config {
            port: 0,
            db_path: ":memory:".into(),
            web_dist: "./web/dist".into(),
            jwt_secret_path: "/tmp/.jwt-test".into(),
            trust_proxy_headers: true,
            max_request_bytes: 64 * 1024 * 1024,
        },
        pool.clone(),
        caches,
        b"test-secret".to_vec(),
        Some(tx),
    );
    let writer = solo_new_api_rust::logging::spawn_writer(pool.clone(), rx);

    let entry = solo_new_api_rust::logging::LogEntry {
        created_at: now,
        user_id: 1,
        username: "root".into(),
        token_name: "t".into(),
        model_name: "m1".into(),
        prompt_tokens: 1,
        completion_tokens: 2,
        use_time_secs: 1,
        is_stream: true,
        channel_id: 3,
        channel_name: "c".into(),
        token_id: 2,
        group: "default".into(),
        ip: "127.0.0.1".into(),
        request_id: String::new(),
        upstream_request_id: String::new(),
        other: "{}".into(),
    };

    put_option(&app, json!({"key": "LogConsumeEnabled", "value": false})).await;
    state.reload_caches().await.unwrap();
    solo_new_api_rust::logging::enqueue(&state, entry.clone()).await;
    // 写日志有 200 毫秒的批量窗口，等它过去再数。
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(log_count(&pool).await, 1, "关闭开关后不该新增日志");

    put_option(&app, json!({"key": "LogConsumeEnabled", "value": true})).await;
    state.reload_caches().await.unwrap();
    solo_new_api_rust::logging::enqueue(&state, entry).await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(log_count(&pool).await, 2, "打开开关后应恢复记录");
    writer.abort();
}

async fn log_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM logs")
        .fetch_one(pool)
        .await
        .unwrap()
}
