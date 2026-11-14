use solo_new_api_rust::{auth::session::verify_password, db};

#[tokio::test]
async fn fresh_database_is_valid_and_seeded_once() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("one-api.db");

    solo_new_api_desktop::bootstrap::ensure_database(&db_path)
        .await
        .unwrap();

    let pool = db::connect(&db_path).await.unwrap();
    db::validate_schema(&pool).await.unwrap();
    let (hash, role): (String, i64) =
        sqlx::query_as("SELECT password, role FROM users WHERE username='root'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(role, 100);
    assert!(verify_password(&hash, "123456"));
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1);
    pool.close().await;

    solo_new_api_desktop::bootstrap::ensure_database(&db_path)
        .await
        .unwrap();
    let pool = db::connect(&db_path).await.unwrap();
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1);
}
