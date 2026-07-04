use crate::app_state::AppState;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, Manager,
};
use tauri_plugin_autostart::ManagerExt;

const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/32x32.png");

/// 创建系统托盘菜单，并把菜单动作接到应用状态。
pub fn setup_tray(app: &mut App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open_settings", "打开设置", true, None::<&str>)?;
    let test = MenuItem::with_id(app, "test_print", "测试打印（未实现）", false, None::<&str>)?;
    let logs = MenuItem::with_id(app, "view_logs", "查看日志", true, None::<&str>)?;
    let restart = MenuItem::with_id(
        app,
        "restart_app",
        "重启应用",
        !cfg!(debug_assertions),
        None::<&str>,
    )?;
    let autostart = MenuItem::with_id(app, "autostart", "开机自启", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &test, &logs, &restart, &autostart, &quit])?;

    let tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("PrintBridge")
        .icon(Image::from_bytes(TRAY_ICON_BYTES)?);

    tray.show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open_settings" | "view_logs" => show_main_window(app),
            "restart_app" => restart_app(app),
            "autostart" => toggle_autostart(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// 如果主设置窗口存在，则显示并聚焦它。
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 切换系统开机自启，并持久化对应应用配置。
fn toggle_autostart(app: &tauri::AppHandle) {
    let autolaunch = app.autolaunch();
    let enabled = match autolaunch.is_enabled() {
        Ok(enabled) => enabled,
        Err(error) => {
            tauri_plugin_log::log::error!("failed to read autostart state: {error}");
            return;
        }
    };

    let next_enabled = !enabled;
    let result = if next_enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };

    if let Err(error) = result {
        tauri_plugin_log::log::error!("failed to update autostart state: {error}");
        return;
    }

    let Some(state) = app
        .try_state::<AppState>()
        .map(|state| state.inner().clone())
    else {
        tauri_plugin_log::log::error!(
            "failed to persist autostart state: app state is not initialized"
        );
        return;
    };

    tauri::async_runtime::spawn(async move {
        let mut config = state.config.read().await.clone();
        config.app.autostart = next_enabled;
        if let Err(error) = state.save_config(config).await {
            tauri_plugin_log::log::error!("failed to persist autostart config: {error}");
        }
    });
}

/// 从托盘菜单重启整个桌面应用。
fn restart_app(app: &tauri::AppHandle) {
    app.request_restart();
}
