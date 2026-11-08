use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use serde::Deserialize;
use serde_json::json;

use crate::error::{ApiError, json_ok};
use crate::state::AppState;

const STRIP_BATCH: i64 = 1_000;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/migrate/new-api", post(migrate))
}

#[derive(Deserialize, Default)]
struct MigrateBody {
    #[serde(default)]
    backup: bool,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// sqlx 的 VACUUM/VACUUM INTO 在这个构建里不落盘，统一用 sqlite3 CLI。
fn run_sqlite(db_path: &Path, sql: &str) -> Result<(), String> {
    let output = Command::new("sqlite3")
        .arg("-cmd")
        .arg(".timeout 10000")
        .arg(db_path)
        .arg(sql)
        .output()
        .map_err(|e| format!("sqlite3 not available: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn db_size(path: &PathBuf) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

async fn other_bytes(pool: &sqlx::SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(SUM(length(other)),0) FROM logs")
        .fetch_one(pool)
        .await
}

async fn migrate(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<MigrateBody>,
) -> Result<axum::response::Response, ApiError> {
    let db_path = PathBuf::from(&state.config.db_path);
    let size_before = db_size(&db_path);
    let other_before = other_bytes(&state.pool).await.map_err(internal)?;

    // 1. 可选：VACUUM INTO 生成一致性备份（在线，不需要停服务）。
    let mut backup_path = String::new();
    if body.backup {
        let name = db_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("one-api.db");
        let backup = db_path.with_file_name(format!("{name}.pre-migrate-{}", now()));
        if backup.exists() {
            return Err(ApiError::new(
                StatusCode::OK,
                "Backup file already exists, retry later",
            ));
        }
        let target = backup.to_string_lossy().replace('\'', "''");
        run_sqlite(&db_path, &format!(".backup '{target}'"))
            .map_err(|e| ApiError::new(StatusCode::OK, format!("Backup failed: {e}")))?;
        backup_path = backup.to_string_lossy().to_string();
    }

    // 2. 去掉历史日志里已不再展示的审计/计费字段，保留缓存与关键渠道信息。
    let mut stripped = 0u64;
    loop {
        let updated = sqlx::query(
            "UPDATE logs SET other = json_remove(other, \
             '$.admin_info.request_policy','$.expr_b64', \
             '$.billing_mode','$.billing_source','$.billing_unit', \
             '$.model_ratio','$.group_ratio','$.completion_ratio','$.cache_ratio', \
             '$.model_price','$.user_group_ratio','$.matched_tier') \
             WHERE id IN (SELECT id FROM logs \
               WHERE other IS NOT NULL AND json_valid(other) AND length(other) > 32 \
               AND (json_type(other,'$.admin_info.request_policy') IS NOT NULL \
                 OR json_type(other,'$.expr_b64') IS NOT NULL \
                 OR json_type(other,'$.billing_mode') IS NOT NULL \
                 OR json_type(other,'$.model_ratio') IS NOT NULL \
                 OR json_type(other,'$.model_price') IS NOT NULL) \
               ORDER BY id LIMIT ?)",
        )
        .bind(STRIP_BATCH)
        .execute(&state.pool)
        .await
        .map_err(internal)?
        .rows_affected();
        stripped += updated;
        if updated == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // 3. 用清理后的日志重建聚合表，再压缩数据库文件。
    let quota = crate::quota_data::backfill(&state.pool)
        .await
        .map_err(internal)?;
    run_sqlite(&db_path, "VACUUM;")
        .map_err(|e| ApiError::new(StatusCode::OK, format!("VACUUM failed: {e}")))?;

    let other_after = other_bytes(&state.pool).await.map_err(internal)?;
    let size_after = db_size(&db_path);
    Ok(json_ok(&json!({
        "backup_path": backup_path,
        "stripped_rows": stripped,
        "other_bytes_before": other_before,
        "other_bytes_after": other_after,
        "db_size_before": size_before,
        "db_size_after": size_after,
        "quota": crate::quota_data::summary_json(&quota),
    })))
}

fn internal(e: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
