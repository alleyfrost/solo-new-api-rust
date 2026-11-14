use solo_new_api_rust::{config, server};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tune_glibc_allocator();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

/// argon2id 的 19 MiB 工作内存与大请求体缓冲都在堆上按次分配；glibc 默认会把
/// 这类大块留在按线程划分的 arena 里，释放后不还给内核（实测 4×8 MiB 请求后
/// RSS 长期停在 170 MiB）。把 mmap 阈值压到 1 MiB，让大块走 mmap/munmap 直接
/// 归还；trim 兜住堆顶空闲页，arena 限 2 避免按线程膨胀。
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn tune_glibc_allocator() {
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 1024 * 1024);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 128 * 1024);
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn tune_glibc_allocator() {}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    solo_new_api_rust::START_TIME.set(start).ok();
    let config = config::Config::from_env();
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!("listening on :{}", config.port);
    match server::serve(config, listener, shutdown_signal()).await {
        Ok(()) => Ok(()),
        Err(server::ServeError::Schema(e)) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
        Err(e) => Err(e.into()),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let term = async {
        use tokio::signal::unix::{signal, SignalKind};
        signal(SignalKind::terminate()).unwrap().recv().await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutdown signal received");
}
