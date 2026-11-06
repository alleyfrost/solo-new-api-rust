use sqlx::SqlitePool;

#[tokio::test]
async fn rejects_empty_database() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let err = solo_new_api_rust::db::validate_schema(&pool)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("users"),
        "missing tables should be reported: {msg}"
    );
}

#[tokio::test]
async fn accepts_minimum_schema() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    for sql in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, display_name TEXT, email TEXT)",
        "CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, expired_time INTEGER, `group` TEXT, unlimited_quota INTEGER, remain_quota INTEGER, used_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, auto_groups TEXT, cross_group_retry INTEGER, deleted_at TEXT)",
        "CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, setting TEXT, param_override TEXT, header_override TEXT, channel_info TEXT, settings TEXT)",
        "CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER, tag TEXT)",
        "CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)",
        "CREATE TABLE user_sessions (sid TEXT PRIMARY KEY, user_id INTEGER, status TEXT, refresh_hash TEXT, expires_at INTEGER, version INTEGER, user_auth_version INTEGER)",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    solo_new_api_rust::db::validate_schema(&pool).await.unwrap();
}
