use std::collections::HashMap;
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::sync::mpsc;

pub const LOG_TYPE_CONSUME: i64 = 2;
pub const LOG_TYPE_ERROR: i64 = 5;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub log_type: i64,
    pub content: Option<String>,
    pub created_at: i64,
    pub user_id: i64,
    pub username: String,
    pub token_name: String,
    pub model_name: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub use_time_secs: i64,
    pub is_stream: bool,
    pub channel_id: i64,
    pub channel_name: String,
    pub token_id: i64,
    pub group: String,
    pub ip: String,
    pub request_id: String,
    pub upstream_request_id: String,
    pub other: String,
}

pub fn channel(capacity: usize) -> (mpsc::Sender<LogEntry>, mpsc::Receiver<LogEntry>) {
    mpsc::channel(capacity)
}

pub fn spawn_writer(
    pool: SqlitePool,
    mut rx: mpsc::Receiver<LogEntry>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut batch: Vec<LogEntry> = Vec::with_capacity(128);
        loop {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(entry)) => {
                    batch.push(entry);
                    while batch.len() < 100 {
                        match rx.try_recv() {
                            Ok(entry) => batch.push(entry),
                            Err(_) => break,
                        }
                    }
                }
                Ok(None) => {
                    flush(&pool, &mut batch).await;
                    break;
                }
                Err(_) => {}
            }
            if !batch.is_empty() {
                flush(&pool, &mut batch).await;
            }
        }
    })
}

/// 按小时桶累计性能指标，供数据看板的“性能健康”读取。
async fn record_perf(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, e: &LogEntry) {
    if e.model_name.is_empty() {
        return;
    }
    let bucket = e.created_at / 3600 * 3600;
    let success = i64::from(e.log_type == LOG_TYPE_CONSUME);
    let latency_ms = e.use_time_secs.max(0) * 1000;
    if let Err(err) = sqlx::query(
        "INSERT INTO perf_metrics (model_name, `group`, bucket_ts, request_count, success_count, total_latency_ms, ttft_sum_ms, ttft_count, output_tokens, generation_ms) \
         VALUES (?,?,?,1,?,?,0,0,?,?) \
         ON CONFLICT(model_name,`group`,bucket_ts) DO UPDATE SET \
           request_count = perf_metrics.request_count + 1, \
           success_count = perf_metrics.success_count + excluded.success_count, \
           total_latency_ms = perf_metrics.total_latency_ms + excluded.total_latency_ms, \
           output_tokens = perf_metrics.output_tokens + excluded.output_tokens, \
           generation_ms = perf_metrics.generation_ms + excluded.generation_ms",
    )
    .bind(&e.model_name)
    .bind(&e.group)
    .bind(bucket)
    .bind(success)
    .bind(latency_ms)
    .bind(e.completion_tokens)
    .bind(latency_ms)
    .execute(&mut **tx)
    .await
    {
        tracing::debug!("perf metrics upsert failed: {err}");
    }
}

struct QuotaDelta {
    user_id: i64,
    username: String,
    model_name: String,
    bucket: i64,
    group: String,
    token_id: i64,
    channel_id: i64,
    token_used: i64,
    count: i64,
    cache_read: i64,
    cache_write: i64,
}

type QuotaKey = (i64, String, String, i64, String, i64, i64);

fn cache_tokens(other: &str) -> (i64, i64) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(other) else {
        return (0, 0);
    };
    let read = value
        .get("cache_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let write = value
        .get("cache_creation_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    (read.max(0), write.max(0))
}

async fn record_quota(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, d: &QuotaDelta) {
    let existing: Result<Option<i64>, _> = sqlx::query_scalar(
        "SELECT id FROM quota_data WHERE user_id=? AND username=? AND model_name=? \
         AND created_at=? AND use_group=? AND token_id=? AND channel_id=? AND node_name='' \
         LIMIT 1",
    )
    .bind(d.user_id)
    .bind(&d.username)
    .bind(&d.model_name)
    .bind(d.bucket)
    .bind(&d.group)
    .bind(d.token_id)
    .bind(d.channel_id)
    .fetch_optional(&mut **tx)
    .await;
    let outcome = match existing {
        Ok(Some(id)) => {
            sqlx::query(
                "UPDATE quota_data SET token_used=token_used+?, count=count+?, quota=quota+0, \
                 cache_read_tokens=cache_read_tokens+?, cache_creation_tokens=cache_creation_tokens+? \
                 WHERE id=?",
            )
            .bind(d.token_used)
            .bind(d.count)
            .bind(d.cache_read)
            .bind(d.cache_write)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map(|_| ())
        }
        Ok(None) => {
            sqlx::query(
                "INSERT INTO quota_data (user_id, username, model_name, created_at, use_group, \
                 token_id, channel_id, node_name, token_used, count, quota, cache_read_tokens, \
                 cache_creation_tokens) VALUES (?,?,?,?,?,?,?, '', ?,?,0,?,?)",
            )
            .bind(d.user_id)
            .bind(&d.username)
            .bind(&d.model_name)
            .bind(d.bucket)
            .bind(&d.group)
            .bind(d.token_id)
            .bind(d.channel_id)
            .bind(d.token_used)
            .bind(d.count)
            .bind(d.cache_read)
            .bind(d.cache_write)
            .execute(&mut **tx)
            .await
            .map(|_| ())
        }
        Err(e) => Err(e),
    };
    if let Err(err) = outcome {
        tracing::debug!("quota data upsert failed: {err}");
    }
}

async fn flush(pool: &SqlitePool, batch: &mut Vec<LogEntry>) {
    if batch.is_empty() {
        return;
    }
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("log tx begin: {e}");
            batch.clear();
            return;
        }
    };
    let mut touched: HashMap<i64, i64> = HashMap::new();
    let mut quota: HashMap<QuotaKey, QuotaDelta> = HashMap::new();
    for e in batch.drain(..) {
        touched
            .entry(e.token_id)
            .and_modify(|t| *t = (*t).max(e.created_at))
            .or_insert(e.created_at);
        record_perf(&mut tx, &e).await;
        if e.log_type == LOG_TYPE_CONSUME {
            let bucket = e.created_at / 3600 * 3600;
            let key = (
                e.user_id,
                e.username.clone(),
                e.model_name.clone(),
                bucket,
                e.group.clone(),
                e.token_id,
                e.channel_id,
            );
            let (cache_read, cache_write) = cache_tokens(&e.other);
            let delta = quota.entry(key).or_insert_with(|| QuotaDelta {
                user_id: e.user_id,
                username: e.username.clone(),
                model_name: e.model_name.clone(),
                bucket,
                group: e.group.clone(),
                token_id: e.token_id,
                channel_id: e.channel_id,
                token_used: 0,
                count: 0,
                cache_read: 0,
                cache_write: 0,
            });
            delta.token_used += e.prompt_tokens.max(0) + e.completion_tokens.max(0);
            delta.count += 1;
            delta.cache_read += cache_read;
            delta.cache_write += cache_write;
        }
        if let Err(err) = sqlx::query(
            "INSERT INTO logs (user_id,created_at,type,content,username,token_name,model_name,quota,prompt_tokens,completion_tokens,use_time,is_stream,channel_id,channel_name,token_id,`group`,ip,request_id,upstream_request_id,other) \
             VALUES (?,?,?,?,?,?,?,0,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(e.user_id)
        .bind(e.created_at)
        .bind(e.log_type)
        .bind(e.content)
        .bind(e.username)
        .bind(e.token_name)
        .bind(e.model_name)
        .bind(e.prompt_tokens)
        .bind(e.completion_tokens)
        .bind(e.use_time_secs)
        .bind(e.is_stream)
        .bind(e.channel_id)
        .bind(e.channel_name)
        .bind(e.token_id)
        .bind(e.group)
        .bind(e.ip)
        .bind(e.request_id)
        .bind(e.upstream_request_id)
        .bind(e.other)
        .execute(&mut *tx)
        .await
        {
            tracing::error!("insert log failed: {err}");
        }
    }
    for delta in quota.values() {
        record_quota(&mut tx, delta).await;
    }
    for (token_id, ts) in touched {
        if let Err(e) = sqlx::query("UPDATE tokens SET accessed_time = ? WHERE id = ?")
            .bind(ts)
            .bind(token_id)
            .execute(&mut *tx)
            .await
        {
            tracing::warn!("update accessed_time failed: {e}");
        }
    }
    if let Err(e) = tx.commit().await {
        tracing::error!("log tx commit: {e}");
    }
}

pub async fn enqueue(state: &crate::state::AppState, entry: LogEntry) {
    // 消费日志受开关控制；错误日志（type=5）始终落库，否则失败请求在
    // 使用记录里完全不可见。
    if entry.log_type == LOG_TYPE_CONSUME {
        let settings = crate::settings::Settings::load(state).await;
        if !settings.log_consume_enabled {
            return;
        }
    }
    let Some(tx) = &state.log_tx else {
        return;
    };
    match tokio::time::timeout(Duration::from_secs(2), tx.send(entry)).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => tracing::warn!("log queue closed; entry dropped"),
        Err(_) => tracing::warn!("log queue full; entry dropped"),
    }
}
