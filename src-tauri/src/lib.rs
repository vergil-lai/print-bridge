pub mod app_state;
pub mod commands;
pub mod config;
pub mod document;
pub mod download;
pub mod logs;
pub mod printing;
pub mod protocol;
pub mod queue;
pub mod remote_client;
pub mod remote_protocol;
pub mod remote_store;
pub mod remote_worker;
pub mod server;
pub mod test_print;
pub mod tray;

use app_state::AppState;
use config::AgentConfig;
use std::io;
#[cfg(target_os = "windows")]
use tauri::path::BaseDirectory;
use tauri::Manager;

/// 启动 Tauri 应用、本地服务和后台打印 worker。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            tray::setup_tray(app)?;

            let app_config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&app_config_dir)?;
            let config_path = app_config_dir.join("config.json");
            let config = AgentConfig::load(&config_path)?;
            let printing = print_backend(app)?;
            let remote_store = remote_store::RemoteStore::open(&app_config_dir.join("remote.sqlite3"))
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            let state = AppState::with_config_path_and_printing(config, config_path, printing)
                .with_remote_store(remote_store);
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
            commands::test_remote_connection,
            commands::get_logs,
            commands::is_debug_build,
            commands::print_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
