use sqlx::SqlitePool;

async fn setup() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE quota_data (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,username TEXT DEFAULT '',model_name TEXT DEFAULT '',created_at INTEGER,use_group TEXT DEFAULT '',token_id INTEGER DEFAULT 0,channel_id INTEGER DEFAULT 0,node_name TEXT DEFAULT '',token_used INTEGER DEFAULT 0,count INTEGER DEFAULT 0,quota INTEGER DEFAULT 0,cache_read_tokens INTEGER DEFAULT 0,cache_creation_tokens INTEGER DEFAULT 0)")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

#[tokio::test]
async fn backfill_rebuilds_the_tail_hour_once() {
    let pool = setup().await;
    let hour = 1_800_000_000i64 / 3600 * 3600;
    // 模拟 Go 已写入的最后一个小时（数值故意不完整，回填应整段重算）。
    sqlx::query("INSERT INTO quota_data (user_id,username,model_name,created_at,use_group,token_id,channel_id,node_name,token_used,count) VALUES (1,'root','m1',?,'default',2,3,'',5,1)")
        .bind(hour)
        .execute(&pool)
        .await
        .unwrap();
    for (tokens, other) in [(10, r#"{"cache_tokens":4}"#), (20, r#"{"cache_tokens":6}"#)] {
        sqlx::query("INSERT INTO logs (user_id,created_at,type,username,model_name,quota,prompt_tokens,completion_tokens,channel_id,token_id,`group`,other) VALUES (1,?,2,'root','m1',0,?,0,3,2,'default',?)")
            .bind(hour + 100 + tokens)
            .bind(tokens)
            .bind(other)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO logs (user_id,created_at,type,username,model_name,quota,prompt_tokens,completion_tokens,channel_id,token_id,`group`,other) VALUES (1,?,5,'root','m1',0,99,0,3,2,'default','{}')")
        .bind(hour + 200)
        .execute(&pool)
        .await
        .unwrap();

    let stats = solo_new_api_rust::quota_data::backfill(&pool)
        .await
        .unwrap();
    assert_eq!(stats.cutoff, hour);
    let row: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT token_used, count, cache_read_tokens, cache_creation_tokens FROM quota_data",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, (30, 2, 10, 0));

    let again = solo_new_api_rust::quota_data::backfill(&pool)
        .await
        .unwrap();
    assert_eq!(again.inserted, 1);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quota_data")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1, "re-running must not duplicate the rebuilt hour");
}
