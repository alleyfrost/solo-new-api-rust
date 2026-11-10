use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 限流等待上限：请求超限后最多排队这么久，仍拿不到就返回 429，不让请求无限挂起。
pub const MAX_WAIT: Duration = Duration::from_secs(60);

/// 令牌桶补液：`tokens` 按每秒 `rate_per_sec` 增长，封顶 `capacity`。
/// 独立成纯函数，方便用合成时间差确定性地测桶的数学，不依赖真实时钟。
pub fn refill_tokens(tokens: f64, capacity: f64, rate_per_sec: f64, elapsed_secs: f64) -> f64 {
    (tokens + elapsed_secs * rate_per_sec).min(capacity)
}

/// 每-token 限流器：RPM 令牌桶 + 并发信号量。0 表示对应维度不限。
pub struct TokenLimiter {
    rpm: u32,
    concurrency: u32,
    bucket: Mutex<Bucket>,
    semaphore: Arc<Semaphore>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl TokenLimiter {
    pub fn new(rpm: i64, concurrency: i64) -> Self {
        let rpm = rpm.max(0) as u32;
        let concurrency = concurrency.max(0) as u32;
        Self {
            rpm,
            concurrency,
            bucket: Mutex::new(Bucket {
                tokens: rpm as f64,
                last: Instant::now(),
            }),
            semaphore: Arc::new(Semaphore::new(concurrency as usize)),
        }
    }

    fn rate_per_sec(&self) -> f64 {
        self.rpm as f64 / 60.0
    }

    /// 限流门：先按 RPM 令牌（不足则睡到补足或超时），再取并发许可。
    /// 并发许可必须持有到请求结束（含流式响应体消费完），由调用方挂进响应体生命周期。
    /// 任一等待超过 `deadline` 返回 Err，调用方转为 429。
    pub async fn gate(&self, deadline: Instant) -> Result<Option<OwnedSemaphorePermit>, ()> {
        if self.rpm > 0 {
            self.wait_rpm_token(deadline).await?;
        }
        if self.concurrency == 0 {
            return Ok(None);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(());
        }
        match tokio::time::timeout(remaining, self.semaphore.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(Some(permit)),
            Ok(Err(_)) | Err(_) => Err(()),
        }
    }

    async fn wait_rpm_token(&self, deadline: Instant) -> Result<(), ()> {
        loop {
            let wait = {
                let mut bucket = self.bucket.lock().expect("bucket lock");
                let now = Instant::now();
                let elapsed = now.duration_since(bucket.last).as_secs_f64();
                bucket.tokens =
                    refill_tokens(bucket.tokens, self.rpm as f64, self.rate_per_sec(), elapsed);
                bucket.last = now;
                if bucket.tokens >= 1.0 {
                    bucket.tokens -= 1.0;
                    None
                } else {
                    Some((1.0 - bucket.tokens) / self.rate_per_sec())
                }
            };
            match wait {
                None => return Ok(()),
                Some(wait) => {
                    if Instant::now() + Duration::from_secs_f64(wait) > deadline {
                        return Err(());
                    }
                    tokio::time::sleep(Duration::from_secs_f64(wait)).await;
                }
            }
        }
    }
}

/// 每-token 限流配置的内存注册表：启动时从 token_rate_limits 加载，
/// 管理端改动时同步更新，/v1 请求按 token_id 查表捡取限流器。
pub struct RateLimitRegistry {
    inner: RwLock<HashMap<i64, Arc<TokenLimiter>>>,
    max_wait: RwLock<Duration>,
}

impl Default for RateLimitRegistry {
    fn default() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            max_wait: RwLock::new(MAX_WAIT),
        }
    }
}

impl RateLimitRegistry {
    pub fn max_wait(&self) -> Duration {
        *self.max_wait.read().expect("registry lock")
    }

    /// 测试注入用：缩短限流等待预算，避免真实 60s 慢测。
    pub fn set_max_wait(&self, max_wait: Duration) {
        *self.max_wait.write().expect("registry lock") = max_wait;
    }
    pub async fn load(&self, pool: &SqlitePool) -> Result<(), sqlx::Error> {
        let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
            "SELECT token_id, rpm, concurrency FROM token_rate_limits \
             WHERE rpm > 0 OR concurrency > 0",
        )
        .fetch_all(pool)
        .await?;
        let mut map = HashMap::new();
        for (token_id, rpm, concurrency) in rows {
            map.insert(token_id, Arc::new(TokenLimiter::new(rpm, concurrency)));
        }
        *self.inner.write().expect("registry lock") = map;
        Ok(())
    }

    pub fn get(&self, token_id: i64) -> Option<Arc<TokenLimiter>> {
        self.inner
            .read()
            .expect("registry lock")
            .get(&token_id)
            .cloned()
    }

    /// 管理端写入后的同步：两个维度都为 0 等价于删除配置。
    pub fn set(&self, token_id: i64, rpm: i64, concurrency: i64) {
        let mut map = self.inner.write().expect("registry lock");
        if rpm > 0 || concurrency > 0 {
            map.insert(token_id, Arc::new(TokenLimiter::new(rpm, concurrency)));
        } else {
            map.remove(&token_id);
        }
    }

    pub fn remove(&self, token_id: i64) {
        self.inner.write().expect("registry lock").remove(&token_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short_deadline(millis: u64) -> Instant {
        Instant::now() + Duration::from_millis(millis)
    }

    #[test]
    fn refill_accumulates_and_caps_at_capacity() {
        assert_eq!(refill_tokens(10.0, 60.0, 1.0, 30.0), 40.0);
        assert_eq!(refill_tokens(40.0, 60.0, 1.0, 30.0), 60.0);
        assert_eq!(refill_tokens(0.0, 1.0, 0.0, 100.0), 0.0);
    }

    #[tokio::test]
    async fn rpm_token_waits_until_refilled_within_deadline() {
        let limiter = TokenLimiter::new(60, 0);
        // 人为把桶打到只剩半个令牌：按 1 token/s 需 0.5s 补足。
        {
            let mut bucket = limiter.bucket.lock().unwrap();
            bucket.tokens = 0.5;
        }
        let start = Instant::now();
        let permit = limiter.gate(short_deadline(2000)).await.unwrap();
        assert!(permit.is_none());
        assert!(start.elapsed() >= Duration::from_millis(450));
    }

    #[tokio::test]
    async fn rpm_exhausted_returns_err_past_deadline() {
        let limiter = TokenLimiter::new(1, 0);
        assert!(limiter.gate(short_deadline(2000)).await.is_ok());
        // 唯一的令牌已消耗，补一个要 60s，远超 100ms 截止时间 → Err（转 429）。
        assert!(limiter.gate(short_deadline(100)).await.is_err());
    }

    #[tokio::test]
    async fn zero_limits_never_block() {
        let limiter = TokenLimiter::new(0, 0);
        assert!(limiter.gate(Instant::now()).await.is_ok());
    }

    #[tokio::test]
    async fn concurrency_holds_until_permit_dropped() {
        let limiter = TokenLimiter::new(0, 1);
        let permit = limiter.gate(short_deadline(2000)).await.unwrap().unwrap();
        assert!(limiter.gate(short_deadline(100)).await.is_err());
        drop(permit);
        assert!(limiter.gate(short_deadline(2000)).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn concurrency_permits_parallel_slots() {
        let limiter = TokenLimiter::new(0, 2);
        let a = limiter.gate(short_deadline(2000)).await.unwrap().unwrap();
        let b = limiter.gate(short_deadline(2000)).await.unwrap().unwrap();
        assert!(limiter.gate(short_deadline(100)).await.is_err());
        drop(a);
        drop(b);
        assert!(limiter.gate(short_deadline(2000)).await.is_ok());
    }

    #[test]
    fn registry_set_remove_and_get() {
        let registry = RateLimitRegistry::default();
        assert!(registry.get(1).is_none());
        registry.set(1, 30, 2);
        assert!(registry.get(1).is_some());
        registry.set(2, 0, 0);
        assert!(registry.get(2).is_none());
        registry.remove(1);
        assert!(registry.get(1).is_none());
    }

    #[tokio::test]
    async fn registry_load_reads_only_limited_rows() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE token_rate_limits (token_id INTEGER PRIMARY KEY, \
             rpm INTEGER NOT NULL DEFAULT 0, concurrency INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO token_rate_limits (token_id, rpm, concurrency) VALUES \
             (1, 60, 3), (2, 0, 1), (3, 0, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let registry = RateLimitRegistry::default();
        registry.load(&pool).await.unwrap();
        assert!(registry.get(1).is_some());
        assert!(registry.get(2).is_some());
        assert!(registry.get(3).is_none());
    }
}
