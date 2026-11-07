#[tokio::test]
async fn batch_writer_inserts_consume_logs() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE perf_metrics (id INTEGER PRIMARY KEY AUTOINCREMENT,model_name TEXT,`group` TEXT,bucket_ts INTEGER,request_count INTEGER DEFAULT 0,success_count INTEGER DEFAULT 0,total_latency_ms INTEGER DEFAULT 0,ttft_sum_ms INTEGER DEFAULT 0,ttft_count INTEGER DEFAULT 0,output_tokens INTEGER DEFAULT 0,generation_ms INTEGER DEFAULT 0)").execute(&pool).await.unwrap();
    sqlx::query("CREATE UNIQUE INDEX idx_perf_model_group_bucket ON perf_metrics (model_name,`group`,bucket_ts)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, accessed_time INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO tokens (id, accessed_time) VALUES (2, 0)")
        .execute(&pool)
        .await
        .unwrap();
    let (tx, rx) = solo_new_api_rust::logging::channel(16);
    let writer = solo_new_api_rust::logging::spawn_writer(pool.clone(), rx);
    for i in 0..2 {
        tx.send(solo_new_api_rust::logging::LogEntry {
            log_type: solo_new_api_rust::logging::LOG_TYPE_CONSUME,
            content: None,
            created_at: 100 + i,
            user_id: 1,
            username: "root".into(),
            token_name: "t".into(),
            model_name: "m".into(),
            prompt_tokens: 10,
            completion_tokens: 5,
            use_time_secs: 1,
            is_stream: true,
            channel_id: 3,
            channel_name: "c".into(),
            token_id: 2,
            group: "default".into(),
            ip: "127.0.0.1".into(),
            request_id: format!("r{i}"),
            upstream_request_id: String::new(),
            other: "{}".into(),
        })
        .await
        .unwrap();
    }
    drop(tx);
    writer.await.unwrap();
    let (count, quota, pt): (i64, i64, i64) =
        sqlx::query_as("SELECT COUNT(*), SUM(quota), SUM(prompt_tokens) FROM logs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((count, quota, pt), (2, 0, 20));
    let accessed: i64 = sqlx::query_scalar("SELECT accessed_time FROM tokens WHERE id = 2")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(accessed, 101);
    let perf: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT SUM(request_count), SUM(success_count), SUM(output_tokens), SUM(generation_ms) FROM perf_metrics",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(perf, (2, 2, 10, 2000));
}
