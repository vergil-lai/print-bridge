use crate::{config::UiLanguage, state::AgentState, test_print::print_calibration_page};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, Manager,
};
use tauri_plugin_autostart::ManagerExt;

const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/32x32.png");

struct TrayLabels {
    open_settings: &'static str,
    test_print: &'static str,
    view_logs: &'static str,
    restart_app: &'static str,
    autostart: &'static str,
    quit: &'static str,
}

struct TrayMenuItems {
    open_settings: MenuItem<tauri::Wry>,
    test_print: MenuItem<tauri::Wry>,
    view_logs: MenuItem<tauri::Wry>,
    restart_app: MenuItem<tauri::Wry>,
    autostart: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

/// 创建系统托盘菜单，并把菜单动作接到应用状态。
pub fn setup_tray(app: &mut App, language: UiLanguage) -> tauri::Result<()> {
    let labels = tray_labels(language);
    let open = MenuItem::with_id(
        app,
        "open_settings",
        labels.open_settings,
        true,
        None::<&str>,
    )?;
    let test = MenuItem::with_id(app, "test_print", labels.test_print, true, None::<&str>)?;
    let logs = MenuItem::with_id(app, "view_logs", labels.view_logs, true, None::<&str>)?;
    let restart = MenuItem::with_id(
        app,
        "restart_app",
        labels.restart_app,
        !cfg!(debug_assertions),
        None::<&str>,
    )?;
    let autostart = MenuItem::with_id(app, "autostart", labels.autostart, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &test, &logs, &restart, &autostart, &quit])?;

    app.manage(TrayMenuItems {
        open_settings: open.clone(),
        test_print: test.clone(),
        view_logs: logs.clone(),
        restart_app: restart.clone(),
        autostart: autostart.clone(),
        quit: quit.clone(),
    });

    let tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("PrintBridge")
        .icon(Image::from_bytes(TRAY_ICON_BYTES)?);

    tray.show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open_settings" | "view_logs" => show_main_window(app),
            "test_print" => test_print(app),
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

/// 按当前界面语言更新已创建的托盘菜单文本。
pub fn apply_tray_language(app: &tauri::AppHandle, language: UiLanguage) -> tauri::Result<()> {
    let Some(items) = app.try_state::<TrayMenuItems>() else {
        return Ok(());
    };
    let labels = tray_labels(language);

    items.open_settings.set_text(labels.open_settings)?;
    items.test_print.set_text(labels.test_print)?;
    items.view_logs.set_text(labels.view_logs)?;
    items.restart_app.set_text(labels.restart_app)?;
    items.autostart.set_text(labels.autostart)?;
    items.quit.set_text(labels.quit)?;

    Ok(())
}

fn tray_labels(language: UiLanguage) -> TrayLabels {
    match language {
        UiLanguage::ZhCn => TrayLabels {
            open_settings: "打开设置",
            test_print: "测试打印",
            view_logs: "查看日志",
            restart_app: "重启应用",
            autostart: "开机自启",
            quit: "退出",
        },
        UiLanguage::En => TrayLabels {
            open_settings: "Open Settings",
            test_print: "Test Print",
            view_logs: "View Logs",
            restart_app: "Restart App",
            autostart: "Launch at Startup",
            quit: "Quit",
        },
    }
}

/// 使用当前默认打印设置提交一张测试校准页。
fn test_print(app: &tauri::AppHandle) {
    let Some(state) = app
        .try_state::<AgentState>()
        .map(|state| state.inner().clone())
    else {
        tauri_plugin_log::log::error!("failed to run test print: app state is not initialized");
        return;
    };

    tauri::async_runtime::spawn(async move {
        if let Err(error) = print_calibration_page(&state).await {
            tauri_plugin_log::log::error!("test print failed: {error}");
        }
    });
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
        .try_state::<AgentState>()
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

#[cfg(test)]
mod tests {
    use super::tray_labels;
    use crate::config::UiLanguage;

    #[test]
    fn tray_labels_returns_chinese_labels() {
        let labels = tray_labels(UiLanguage::ZhCn);

        assert_eq!(labels.open_settings, "打开设置");
        assert_eq!(labels.test_print, "测试打印");
        assert_eq!(labels.view_logs, "查看日志");
        assert_eq!(labels.restart_app, "重启应用");
        assert_eq!(labels.autostart, "开机自启");
        assert_eq!(labels.quit, "退出");
    }

    #[test]
    fn tray_labels_returns_english_labels() {
        let labels = tray_labels(UiLanguage::En);

        assert_eq!(labels.open_settings, "Open Settings");
        assert_eq!(labels.test_print, "Test Print");
        assert_eq!(labels.view_logs, "View Logs");
        assert_eq!(labels.restart_app, "Restart App");
        assert_eq!(labels.autostart, "Launch at Startup");
        assert_eq!(labels.quit, "Quit");
    }
}
