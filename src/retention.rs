use std::time::Duration;

use crate::settings::Settings;
use crate::state::AppState;

const CLEAN_BATCH: i64 = 5_000;
const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 3600);

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 按系统设置的天数清理过期日志：启动后 1 分钟跑一次，之后每 6 小时一次。
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let days = Settings::load(&state).await.log_retention_days;
            if days > 0 {
                let cutoff = now() - days * 86400;
                let mut total = 0i64;
                loop {
                    match crate::routes::admin::log::delete_logs_before(
                        &state.pool,
                        cutoff,
                        CLEAN_BATCH,
                    )
                    .await
                    {
                        Ok(0) => break,
                        Ok(deleted) => {
                            total += deleted;
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        Err(e) => {
                            tracing::warn!("log retention cleanup failed: {e}");
                            break;
                        }
                    }
                }
                if total > 0 {
                    tracing::info!("log retention removed {total} rows older than {days} days");
                }
            }
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    });
}
