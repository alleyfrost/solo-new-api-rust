use std::fmt;
use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, SqlitePool};

#[derive(Debug)]
pub struct SchemaError {
    pub missing: Vec<String>,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "database schema mismatch, missing: {}",
            self.missing.join(", ")
        )
    }
}

impl std::error::Error for SchemaError {}

pub async fn connect(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_millis(5000))
        .foreign_keys(true)
        // 每连接 1 MiB 页缓存；单用户读多写少，压低连接数即可控住常驻内存。
        .pragma("cache_size", "-1024")
        .disable_statement_logging();
    SqlitePoolOptions::new()
        .max_connections(8)
        .min_connections(1)
        .idle_timeout(std::time::Duration::from_secs(60))
        .max_lifetime(std::time::Duration::from_secs(1800))
        .connect_with(opts)
        .await
}

pub async fn validate_schema(pool: &SqlitePool) -> Result<(), SchemaError> {
    const REQUIRED: &[(&str, &[&str])] = &[
        (
            "users",
            &[
                "id",
                "username",
                "password",
                "role",
                "status",
                "group",
                "auth_version",
            ],
        ),
        (
            "tokens",
            &[
                "id",
                "user_id",
                "key",
                "status",
                "expired_time",
                "group",
                "unlimited_quota",
                "deleted_at",
            ],
        ),
        (
            "channels",
            &[
                "id", "type", "key", "status", "models", "group", "priority", "weight",
            ],
        ),
        (
            "abilities",
            &[
                "group",
                "model",
                "channel_id",
                "enabled",
                "priority",
                "weight",
            ],
        ),
        ("options", &["key", "value"]),
        (
            "user_sessions",
            &[
                "sid",
                "user_id",
                "status",
                "refresh_hash",
                "expires_at",
                "version",
                "user_auth_version",
            ],
        ),
    ];
    let mut missing = Vec::new();
    for (table, columns) in REQUIRED {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table)
                .fetch_optional(pool)
                .await
                .unwrap_or(None);
        if exists.is_none() {
            missing.push(format!("table {table}"));
            continue;
        }
        for column in *columns {
            let found: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM pragma_table_info(?) WHERE name=?")
                    .bind(table)
                    .bind(column)
                    .fetch_optional(pool)
                    .await
                    .unwrap_or(None);
            if found.is_none() {
                missing.push(format!("{table}.{column}"));
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(SchemaError { missing })
    }
}
