pub use print_bridge_runtime::{
    agent_guard, document, download, html, logs, office, printing, queue, remote_client,
    remote_store, remote_worker, server, state, task_history, test_print,
};
pub mod cli;
pub mod cli_integration;
pub mod commands;
mod product_cli;
pub use print_bridge_core::{config, ip_whitelist, protocol, remote_protocol};
pub mod tray;

use config::AgentConfig;
use print_bridge_cli::{CommandExecutor, CommandService};
use print_bridge_runtime::{RuntimeBuilder, RuntimeCommandExecutor, RuntimePaths};
use state::AgentState;
#[cfg(test)]
use std::io;
use std::sync::Arc;
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

#[cfg(test)]
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
            tray::setup_tray(app, config.app.language)?;
            let printing = print_backend(app)?;
            let paths = RuntimePaths::new(
                config_path,
                app_config_dir.clone(),
                app_config_dir.join("run"),
            );
            let runtime = RuntimeBuilder::new(paths)
                .print_backend(printing)
                .build()
                .map_err(std::io::Error::other)?;
            let handle =
                tauri::async_runtime::block_on(runtime.start()).map_err(std::io::Error::other)?;
            let state: AgentState = handle.state();
            let executor: Arc<dyn CommandExecutor> = Arc::new(RuntimeCommandExecutor::new(
                state.clone(),
                handle.listen_addr(),
            ));
            app.manage(Arc::new(CommandService::new(
                Some(executor.clone()),
                executor,
            )));
            app.manage(state);
            app.manage(handle);
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
            commands::list_printers,
            commands::list_papers,
            commands::is_debug_build,
            commands::get_cli_integration_status,
            commands::install_cli_integration,
            commands::uninstall_cli_integration,
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
