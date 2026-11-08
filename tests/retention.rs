use sqlx::SqlitePool;

#[tokio::test]
async fn deletes_only_rows_older_than_the_cutoff() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,created_at INTEGER)")
        .execute(&pool)
        .await
        .unwrap();
    for ts in [100, 200, 300, 400] {
        sqlx::query("INSERT INTO logs (created_at) VALUES (?)")
            .bind(ts)
            .execute(&pool)
            .await
            .unwrap();
    }
    let deleted = solo_new_api_rust::routes::admin::log::delete_logs_before(&pool, 300, 10)
        .await
        .unwrap();
    assert_eq!(deleted, 2);
    let left: Vec<i64> = sqlx::query_scalar("SELECT created_at FROM logs ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, vec![300, 400]);
}
