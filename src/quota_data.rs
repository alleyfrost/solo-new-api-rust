use serde_json::Value;
use sqlx::SqlitePool;

/// 聚合表缺失（旧测试库）时直接跳过，不影响服务启动。
pub struct BackfillStats {
    pub cutoff: i64,
    pub deleted: u64,
    pub inserted: u64,
}

fn empty() -> BackfillStats {
    BackfillStats {
        cutoff: 0,
        deleted: 0,
        inserted: 0,
    }
}

/// 以 `logs` 为事实来源修复 `quota_data`：
/// - 从聚合表最后一个小时开始（该小时整段重算，避免小时内的重复计数）
/// - 保留更早的 Go 聚合行，只重建之后的小时
pub async fn backfill(pool: &SqlitePool) -> Result<BackfillStats, sqlx::Error> {
    let table: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='quota_data'",
    )
    .fetch_optional(pool)
    .await?;
    if table.is_none() {
        return Ok(empty());
    }
    let min_log: Option<i64> = sqlx::query_scalar("SELECT MIN(created_at) FROM logs WHERE type=2")
        .fetch_one(pool)
        .await?;
    let Some(min_log) = min_log else {
        return Ok(empty());
    };
    let max_bucket: Option<i64> = sqlx::query_scalar("SELECT MAX(created_at) FROM quota_data")
        .fetch_one(pool)
        .await?;
    let cutoff = match max_bucket {
        Some(max) => max / 3600 * 3600,
        None => min_log / 3600 * 3600,
    };
    let deleted = if max_bucket.is_some() {
        sqlx::query("DELETE FROM quota_data WHERE created_at >= ?")
            .bind(cutoff)
            .execute(pool)
            .await?
            .rows_affected()
    } else {
        0
    };
    let inserted = sqlx::query(
        "INSERT INTO quota_data \
         (user_id, username, model_name, created_at, use_group, token_id, channel_id, node_name, \
          token_used, count, quota, cache_read_tokens, cache_creation_tokens) \
         SELECT user_id, COALESCE(username,''), COALESCE(model_name,''), created_at/3600*3600, \
                COALESCE(`group`,''), COALESCE(token_id,0), COALESCE(channel_id,0), '', \
                SUM(MAX(prompt_tokens,0)+MAX(completion_tokens,0)), COUNT(*), SUM(quota), \
                SUM(CASE WHEN json_valid(other) THEN COALESCE(json_extract(other,'$.cache_tokens'),0) ELSE 0 END), \
                SUM(CASE WHEN json_valid(other) THEN COALESCE(json_extract(other,'$.cache_creation_tokens'),0) ELSE 0 END) \
         FROM logs WHERE type=2 AND created_at >= ? \
         GROUP BY user_id, username, model_name, created_at/3600, `group`, token_id, channel_id",
    )
    .bind(cutoff)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(BackfillStats {
        cutoff,
        deleted,
        inserted,
    })
}

pub fn summary_json(stats: &BackfillStats) -> Value {
    serde_json::json!({
        "cutoff": stats.cutoff,
        "deleted": stats.deleted,
        "inserted": stats.inserted,
    })
}
