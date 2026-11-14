use std::path::{Path, PathBuf};

use solo_new_api_rust::{auth::session::hash_password, db};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

const SCHEMA_SQL: &str = include_str!("../schema/one-api.sql");

pub struct Paths {
    pub db: PathBuf,
    pub jwt_secret: PathBuf,
}

pub fn prepare(app_data_dir: &Path) -> Result<Paths, String> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    Ok(Paths {
        db: app_data_dir.join("one-api.db"),
        jwt_secret: app_data_dir.join(".rust_jwt_secret"),
    })
}

/// 数据目录下已有库时只做结构校验，绝不迁移；不存在时建 new-api 兼容结构并
/// 种子默认管理员。桌面库独立于服务器 /data 目录，不影响现有部署。
pub async fn ensure_database(db_path: &Path) -> Result<(), String> {
    if db_path.exists() {
        let pool = db::connect(db_path)
            .await
            .map_err(|e| format!("打开数据库失败: {e}"))?;
        let result = db::validate_schema(&pool).await;
        pool.close().await;
        return result.map_err(|e| format!("数据库结构不兼容: {e}"));
    }

    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_millis(5000));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|e| format!("创建数据库失败: {e}"))?;

    let hash = hash_password("123456").map_err(|e| format!("生成默认口令失败: {e:?}"))?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::raw_sql(SCHEMA_SQL)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("初始化表结构失败: {e}"))?;
    sqlx::query(
        "INSERT INTO users (username, password, display_name, role, status, `group`, auth_version, created_at) \
         VALUES ('root', ?, 'root', 100, 1, 'default', 1, strftime('%s','now'))",
    )
    .bind(hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("创建默认管理员失败: {e}"))?;
    tx.commit().await.map_err(|e| e.to_string())?;
    pool.close().await;
    Ok(())
}
