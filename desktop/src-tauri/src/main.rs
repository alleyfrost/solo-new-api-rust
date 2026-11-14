#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use solo_new_api_desktop::{bootstrap, server_task, server_task::ServerHandle, tray};
use solo_new_api_rust::config::Config;
use tauri::webview::NewWindowResponse;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

static EXITING: AtomicBool = AtomicBool::new(false);

struct AppState {
    server: Mutex<Option<ServerHandle>>,
}

fn main() {
    // 桌面版免登录：服务端只在回环监听时接受该开关，Web/Docker 部署始终正常登录。
    // 此时尚未创建任何线程，进程启动阶段设置环境变量是安全的。
    unsafe {
        std::env::set_var("DESKTOP_DISABLE_LOGIN", "1");
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = match app.path().app_data_dir() {
                Ok(dir) => dir,
                Err(e) => fatal(app, "无法定位数据目录", &e.to_string()),
            };
            let paths = match bootstrap::prepare(&data_dir) {
                Ok(paths) => paths,
                Err(e) => fatal(app, "初始化数据目录失败", &e),
            };
            let config = Config {
                port: 3000,
                db_path: paths.db,
                web_dist: resolve_web_dist(app),
                jwt_secret_path: paths.jwt_secret,
                trust_proxy_headers: false,
                max_request_bytes: 33_554_432,
            };
            let server = match server_task::start(config) {
                Ok(server) => server,
                Err(e) => fatal(app, "服务启动失败", &e),
            };
            let url: tauri::Url = format!("http://127.0.0.1:{}", server.addr.port())
                .parse()
                .expect("valid local url");
            let navigation = app.handle().clone();
            let new_window = app.handle().clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("New API Desktop")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 600.0)
                .center()
                .on_navigation(move |url| {
                    if url.host_str() == Some("127.0.0.1") {
                        return true;
                    }
                    if matches!(url.scheme(), "http" | "https") {
                        let _ = navigation.opener().open_url(url.as_str(), None::<&str>);
                    }
                    false
                })
                .on_new_window(move |url, _features| {
                    if matches!(url.scheme(), "http" | "https") {
                        let _ = new_window.opener().open_url(url.as_str(), None::<&str>);
                    }
                    NewWindowResponse::Deny
                })
                .build()?;
            app.manage(AppState {
                server: Mutex::new(Some(server)),
            });
            tray::create(app)?;
            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => tray::show(app),
            "hide" => tray::hide(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                if EXITING.swap(true, Ordering::SeqCst) {
                    return;
                }
                api.prevent_exit();
                let handle = app.clone();
                std::thread::spawn(move || {
                    if let Some(state) = handle.try_state::<AppState>()
                        && let Some(server) = state.server.lock().unwrap().take()
                    {
                        let _ = server.shutdown.send(());
                        let _ = server.join.join();
                    }
                    handle.exit(0);
                });
            }
        });
}

fn resolve_web_dist(app: &tauri::App) -> PathBuf {
    if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist")
    } else {
        app.path()
            .resource_dir()
            .map(|dir| dir.join("web-dist"))
            .unwrap_or_else(|_| PathBuf::from("web-dist"))
    }
}

fn fatal(app: &tauri::App, title: &str, message: &str) -> ! {
    let handle = app.handle().clone();
    let title = title.to_owned();
    let message = message.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        handle
            .dialog()
            .message(message)
            .title(title)
            .kind(MessageDialogKind::Error)
            .blocking_show();
        let _ = tx.send(());
    });
    let _ = rx.recv();
    std::process::exit(1);
}
