pub mod agent_guard;
pub mod app_state;
pub mod cli;
pub mod commands;
pub mod config;
pub mod config_transfer;
pub mod document;
pub mod download;
pub mod html;
pub mod ip_whitelist;
pub mod logs;
pub mod office;
pub mod printing;
pub mod protocol;
pub mod queue;
pub mod remote_client;
pub mod remote_protocol;
pub mod remote_store;
pub mod remote_worker;
pub mod runtime;
pub mod server;
pub mod service_manager;
pub mod task_history;
pub mod test_print;
pub mod tray;

use app_state::AppState;
use config::AgentConfig;
use html::{browser::BrowserHtmlRenderer, resource_policy::ResourcePolicy};
use std::{io, sync::Arc};
#[cfg(target_os = "windows")]
use tauri::path::BaseDirectory;
use tauri::Manager;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeMode {
    Gui,
    Headless,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimePlatform {
    Windows,
    Macos,
    Linux,
    Other,
}

/// 原生 WebView fallback 已因无法统一证明其资源隔离而禁用。
#[cfg(test)]
const fn uses_gui_webview_fallback(mode: RuntimeMode, platform: RuntimePlatform) -> bool {
    let _ = (mode, platform);
    false
}

fn existing_agent_startup_error(status: agent_guard::AgentPortStatus) -> Option<io::Error> {
    match status {
        agent_guard::AgentPortStatus::Available => None,
        agent_guard::AgentPortStatus::PrintBridge(agent) => Some(io::Error::new(
            io::ErrorKind::AlreadyExists,
            agent_guard::already_running_message(&agent),
        )),
        agent_guard::AgentPortStatus::OccupiedByOther { addr } => Some(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("local service port is already occupied at {addr}"),
        )),
    }
}

/// 启动 Tauri 应用、本地服务和后台打印 worker。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&app_config_dir)?;
            let config_path = app_config_dir.join("config.json");
            let config = AgentConfig::load(&config_path)?;
            if let Some(error) =
                existing_agent_startup_error(agent_guard::check_agent_port(&config))
            {
                tauri_plugin_log::log::error!("{error}");
                return Err(error.into());
            }
            tray::setup_tray(app, config.app.language)?;
            let printing = print_backend(app)?;
            let remote_store =
                remote_store::RemoteStore::open(&app_config_dir.join("remote.sqlite3"))
                    .map_err(io::Error::other)?;
            let task_history =
                task_history::TaskHistoryStore::open(&app_config_dir.join("task_history.sqlite3"))
                    .map_err(io::Error::other)?;
            let state = AppState::with_config_path_printing_and_html_renderer(
                config,
                config_path,
                printing,
                Arc::new(BrowserHtmlRenderer::new(ResourcePolicy::system())),
            )
            .with_remote_store(remote_store)
            .with_task_history_store(task_history);
            let server_state = state.clone();
            let worker_state = state.clone();
            let remote_worker_state = state.clone();

            // 让本地服务和队列 worker 运行在 UI 线程之外。
            tauri::async_runtime::spawn(async move {
                if let Err(error) = server::run_server(server_state).await {
                    tauri_plugin_log::log::error!("local server stopped: {error}");
                }
            });

            tauri::async_runtime::spawn(async move {
                queue::run_worker(worker_state).await;
            });

            tauri::async_runtime::spawn(async move {
                remote_worker::run_worker(remote_worker_state).await;
            });

            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::export_config_file,
            commands::preview_config_import,
            commands::import_config_file,
            commands::test_remote_connection,
            commands::get_logs,
            commands::get_task_history,
            commands::get_task_history_events,
            commands::clear_task_history,
            commands::is_debug_build,
            commands::print_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 从当前进程参数运行 CLI，供独立二进制入口调用。
pub fn run_cli_from_env() -> i32 {
    cli::run_cli_from_env()
}

/// 选择当前平台的打印后端，并在需要时注入打包资源。
fn print_backend(app: &tauri::App) -> tauri::Result<Box<dyn printing::PrintBackend + Send + Sync>> {
    #[cfg(target_os = "windows")]
    {
        let sumatra_path = app
            .path()
            .resolve("SumatraPDF.exe", BaseDirectory::Resource)?;
        Ok(printing::windows_backend(sumatra_path))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Ok(printing::default_backend())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_guard::{AgentPortStatus, RunningAgent};

    #[test]
    fn gui_runtime_never_uses_webview_fallback() {
        for platform in [
            RuntimePlatform::Windows,
            RuntimePlatform::Macos,
            RuntimePlatform::Linux,
            RuntimePlatform::Other,
        ] {
            assert!(!uses_gui_webview_fallback(RuntimeMode::Gui, platform));
        }
    }

    #[test]
    fn headless_runtime_never_uses_webview_fallback() {
        for platform in [
            RuntimePlatform::Windows,
            RuntimePlatform::Macos,
            RuntimePlatform::Linux,
            RuntimePlatform::Other,
        ] {
            assert!(!uses_gui_webview_fallback(RuntimeMode::Headless, platform));
        }
    }

    #[test]
    fn existing_agent_startup_error_allows_available_port() {
        assert!(existing_agent_startup_error(AgentPortStatus::Available).is_none());
    }

    #[test]
    fn existing_agent_startup_error_blocks_print_bridge_agent() {
        let error = existing_agent_startup_error(AgentPortStatus::PrintBridge(RunningAgent {
            addr: "127.0.0.1:17890".parse().unwrap(),
        }))
        .unwrap();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            error.to_string(),
            "PrintBridge Agent is already running at 127.0.0.1:17890"
        );
    }

    #[test]
    fn existing_agent_startup_error_blocks_other_occupied_port() {
        let error = existing_agent_startup_error(AgentPortStatus::OccupiedByOther {
            addr: "127.0.0.1:17890".parse().unwrap(),
        })
        .unwrap();

        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(
            error.to_string(),
            "local service port is already occupied at 127.0.0.1:17890"
        );
    }
}
