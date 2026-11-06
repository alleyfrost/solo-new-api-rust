use sqlx::SqlitePool;

#[tokio::test]
async fn loads_users_tokens_channels_and_abilities() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users VALUES (1,'root','hash','Root',100,1,'default',1,NULL,NULL)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO tokens VALUES (2,1,'abc123',1,'t',-1,0,1,0,'','', 'vip',NULL,0,NULL)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels VALUES (3,1,'sk-up',1,'c','m1,m2','vip',NULL,0,0,'{}',NULL,NULL)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO abilities VALUES ('vip','m1',3,1,0,0)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO options VALUES ('RetryTimes','10')").execute(&pool).await.unwrap();

    let caches = solo_new_api_rust::state::Caches::load(&pool).await.unwrap();
    assert_eq!(caches.users.len(), 1);
    assert_eq!(caches.token_by_key("abc123").unwrap().group, "vip");
    assert_eq!(caches.channels.len(), 1);
    assert_eq!(caches.abilities[&("vip".to_string(), "m1".to_string())].len(), 1);
    assert_eq!(caches.options["RetryTimes"], "10");
}
