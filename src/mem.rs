use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::state::AppState;

const BIG_BODY_BYTES: usize = 2 * 1024 * 1024;
const HUGE_BODY_BYTES: usize = 8 * 1024 * 1024;

/// 大请求体的并发闸门：单实例内存上限不随客户端并发线性增长。
/// - 2–8 MiB：最多 2 个并发
/// - > 8 MiB：最多 1 个并发（其余请求在 body 读取前排队，形成背压）
pub struct BodyGate {
    pub big: Arc<Semaphore>,
    pub huge: Arc<Semaphore>,
}

impl Default for BodyGate {
    fn default() -> Self {
        Self {
            big: Arc::new(Semaphore::new(2)),
            huge: Arc::new(Semaphore::new(1)),
        }
    }
}

pub async fn large_body_gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let len = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let permit: Option<OwnedSemaphorePermit> = if len > HUGE_BODY_BYTES {
        state.body_gate.huge.clone().acquire_owned().await.ok()
    } else if len > BIG_BODY_BYTES {
        state.body_gate.big.clone().acquire_owned().await.ok()
    } else {
        None
    };
    let response = next.run(req).await;
    drop(permit);
    if len > BIG_BODY_BYTES {
        trim_heap();
    }
    response
}

/// 周期性归还空闲堆页，覆盖未触发闸门的中等请求与长期运行碎片。
pub fn spawn_periodic_trim() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            trim_heap();
        }
    });
}

/// 大请求结束后尽快把 glibc 空闲页还给内核；限频避免影响正常请求。
pub fn trim_heap() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        static LAST: AtomicU64 = AtomicU64::new(0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let last = LAST.load(Ordering::Relaxed);
        if now.saturating_sub(last) < 5 {
            return;
        }
        if LAST
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        tokio::task::spawn_blocking(|| unsafe {
            libc::malloc_trim(0);
        });
    }
}
