use std::collections::HashMap;
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct LogEntry {
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
    for e in batch.drain(..) {
        touched
            .entry(e.token_id)
            .and_modify(|t| *t = (*t).max(e.created_at))
            .or_insert(e.created_at);
        if let Err(err) = sqlx::query(
            "INSERT INTO logs (user_id,created_at,type,content,username,token_name,model_name,quota,prompt_tokens,completion_tokens,use_time,is_stream,channel_id,channel_name,token_id,`group`,ip,request_id,upstream_request_id,other) \
             VALUES (?,?,2,NULL,?,?,?,0,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(e.user_id)
        .bind(e.created_at)
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
    let Some(tx) = &state.log_tx else {
        return;
    };
    match tokio::time::timeout(Duration::from_secs(2), tx.send(entry)).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => tracing::warn!("log queue closed; entry dropped"),
        Err(_) => tracing::warn!("log queue full; entry dropped"),
    }
}
