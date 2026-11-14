use std::net::SocketAddr;
use std::sync::mpsc;
use std::time::Duration;

use solo_new_api_rust::config::Config;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::bootstrap;

pub struct ServerHandle {
    pub addr: SocketAddr,
    pub shutdown: oneshot::Sender<()>,
    pub join: std::thread::JoinHandle<()>,
}

/// 在独立线程里跑 tokio：先确保数据库就绪，再绑定本地端口，最后进入服务循环。
/// 就绪（或失败）经同步通道回传，调用方据此决定建窗口还是提示错误。
pub fn start(config: Config) -> Result<ServerHandle, String> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<SocketAddr, String>>();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let join = std::thread::Builder::new()
        .name("api-server".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("创建运行时失败: {e}")));
                    return;
                }
            };
            runtime.block_on(async move {
                if let Err(e) = bootstrap::ensure_database(&config.db_path).await {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
                let listener = match bind_preferred().await {
                    Ok(listener) => listener,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let addr = match listener.local_addr() {
                    Ok(addr) => addr,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("读取监听地址失败: {e}")));
                        return;
                    }
                };
                if ready_tx.send(Ok(addr)).is_err() {
                    return;
                }
                let shutdown = async move {
                    let _ = shutdown_rx.await;
                };
                if let Err(e) = solo_new_api_rust::server::serve(config, listener, shutdown).await {
                    eprintln!("server stopped: {e}");
                }
            });
        })
        .map_err(|e| format!("启动服务线程失败: {e}"))?;
    let addr = ready_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|e| format!("服务启动超时: {e}"))??;
    Ok(ServerHandle {
        addr,
        shutdown: shutdown_tx,
        join,
    })
}

/// 桌面只监听回环地址：优先 PORT 环境变量，其次 3000..=3009，全部占用则交给
/// 系统分配，避免与常见开发端口冲突。
async fn bind_preferred() -> Result<TcpListener, String> {
    if let Ok(raw) = std::env::var("PORT") {
        let port: u16 = raw.parse().map_err(|_| format!("PORT 无效: {raw}"))?;
        return bind(port)
            .await
            .map_err(|e| format!("端口 {port} 不可用: {e}"));
    }
    for port in 3000..=3009 {
        if let Ok(listener) = bind(port).await {
            return Ok(listener);
        }
    }
    bind(0).await.map_err(|e| format!("无法绑定本地端口: {e}"))
}

async fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).await
}
