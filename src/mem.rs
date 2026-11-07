use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::Notify;

use crate::settings::{DEFAULT_BODY_GATE_BIG, DEFAULT_BODY_GATE_HUGE};
use crate::state::AppState;

const BIG_BODY_BYTES: usize = 2 * 1024 * 1024;
const HUGE_BODY_BYTES: usize = 8 * 1024 * 1024;

/// 可动态调整上限的并发闸门；等待者通过 Notify 唤醒，不轮询。
pub struct DynamicGate {
    limit: AtomicUsize,
    in_flight: AtomicUsize,
    notify: Notify,
}

impl DynamicGate {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: AtomicUsize::new(limit.max(1)),
            in_flight: AtomicUsize::new(0),
            notify: Notify::new(),
        }
    }

    pub fn limit(&self) -> usize {
        self.limit.load(Ordering::Acquire).max(1)
    }

    pub fn set_limit(&self, limit: usize) {
        self.limit.store(limit.max(1), Ordering::Release);
        self.notify.notify_waiters();
    }

    pub async fn acquire(&self) -> GateGuard<'_> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let limit = self.limit();
            if self.in_flight.fetch_add(1, Ordering::AcqRel) < limit {
                return GateGuard { gate: self };
            }
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            notified.await;
        }
    }
}

pub struct GateGuard<'a> {
    gate: &'a DynamicGate,
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        self.gate.in_flight.fetch_sub(1, Ordering::AcqRel);
        self.gate.notify.notify_waiters();
    }
}

/// 大请求体的并发闸门：单实例内存上限不随客户端并发线性增长。
/// - 2–8 MiB：默认 8 个并发
/// - 8 MiB 以上：默认 4 个并发（其余请求在 body 读取前排队，形成背压）
///
/// 两个上限都可从系统设置动态调整。
pub struct BodyGate {
    pub big: DynamicGate,
    pub huge: DynamicGate,
}

impl BodyGate {
    pub fn new(big: usize, huge: usize) -> Self {
        Self {
            big: DynamicGate::new(big),
            huge: DynamicGate::new(huge),
        }
    }

    pub fn set_limits(&self, big: usize, huge: usize) {
        self.big.set_limit(big);
        self.huge.set_limit(huge);
    }
}

impl Default for BodyGate {
    fn default() -> Self {
        Self::new(DEFAULT_BODY_GATE_BIG, DEFAULT_BODY_GATE_HUGE)
    }
}

pub async fn large_body_gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let len = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let guard = if len > HUGE_BODY_BYTES {
        Some(state.body_gate.huge.acquire().await)
    } else if len > BIG_BODY_BYTES {
        Some(state.body_gate.big.acquire().await)
    } else {
        None
    };
    let response = next.run(req).await;
    drop(guard);
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
