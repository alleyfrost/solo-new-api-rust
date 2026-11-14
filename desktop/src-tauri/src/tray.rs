use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

pub fn create(app: &tauri::App) -> tauri::Result<()> {
    let (show_label, hide_label, quit_label) = labels();
    let show = MenuItem::with_id(app, "show", show_label, true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", hide_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
    let icon = app
        .default_window_icon()
        .expect("bundled window icon is missing")
        .clone();

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn toggle(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn labels() -> (&'static str, &'static str, &'static str) {
    if chinese_locale() {
        ("显示主窗口", "隐藏到托盘", "退出")
    } else {
        ("Show Window", "Hide to Tray", "Quit")
    }
}

/// GUI 应用在 macOS 上不继承 shell 的 LANG，需要读系统偏好；其余平台看 LANG。
fn chinese_locale() -> bool {
    if std::env::var("LANG")
        .map(|value| value.to_lowercase().starts_with("zh"))
        .unwrap_or(false)
    {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
        {
            if String::from_utf8_lossy(&output.stdout).contains("zh") {
                return true;
            }
        }
    }
    false
}
