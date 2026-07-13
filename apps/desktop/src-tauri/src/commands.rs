use crate::{
    cli_integration::{self, CliIntegrationStatus},
    config::{AgentConfig, MAX_SERVICE_PORT, MIN_SERVICE_PORT},
    logs::TaskLogEntry,
    state::AgentState,
    task_history::{TaskHistoryEvent, TaskHistoryJob},
    tray::apply_tray_language,
};
use print_bridge_cli::{
    config_transfer::{ExportConfigOptions, ImportPreview},
    Command, CommandError, CommandResult, CommandService,
};
use print_bridge_core::printing::{PaperInfo, PrinterInfo};
use std::{net::TcpListener, path::PathBuf, sync::Arc};
use tauri::State;
use tauri_plugin_autostart::ManagerExt;

/// 返回当前 Agent 配置给 Tauri 前端。
#[tauri::command]
pub async fn get_config(
    service: State<'_, Arc<CommandService>>,
) -> Result<AgentConfig, CommandError> {
    match service.execute(Command::GetConfig).await? {
        CommandResult::Config(config) => Ok(*config),
        _ => unreachable!("GetConfig returned an unexpected result"),
    }
}

/// 返回当前运行的桌面应用是否为 debug 构建。
#[tauri::command]
pub fn is_debug_build() -> bool {
    cfg!(debug_assertions)
}

/// 返回当前平台的命令行工具安装状态。
#[tauri::command]
pub async fn get_cli_integration_status() -> Result<CliIntegrationStatus, CommandError> {
    run_cli_integration(cli_integration::status).await
}

/// 安装当前用户可管理的命令行工具入口。
#[tauri::command]
pub async fn install_cli_integration() -> Result<CliIntegrationStatus, CommandError> {
    run_cli_integration(cli_integration::install).await
}

/// 移除当前用户可管理的命令行工具入口。
#[tauri::command]
pub async fn uninstall_cli_integration() -> Result<CliIntegrationStatus, CommandError> {
    run_cli_integration(cli_integration::uninstall).await
}

async fn run_cli_integration(
    operation: fn() -> Result<CliIntegrationStatus, String>,
) -> Result<CliIntegrationStatus, CommandError> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| {
            CommandError::new(
                print_bridge_cli::CommandErrorKind::Runtime,
                error.to_string(),
            )
        })?
        .map_err(|error| CommandError::new(print_bridge_cli::CommandErrorKind::Runtime, error))
}

/// 应用应用层设置，并保存完整 Agent 配置。
#[tauri::command]
pub async fn save_config(
    app: tauri::AppHandle,
    config: AgentConfig,
    state: State<'_, AgentState>,
    service: State<'_, Arc<CommandService>>,
) -> Result<AgentConfig, CommandError> {
    let current_port = state.config.read().await.service.port;
    if config.service.port != current_port {
        check_service_port_available_for_host("0.0.0.0", config.service.port).map_err(
            |message| CommandError::new(print_bridge_cli::CommandErrorKind::InvalidInput, message),
        )?;
    }

    apply_autostart(&app, config.app.autostart).map_err(|message| {
        CommandError::new(print_bridge_cli::CommandErrorKind::Runtime, message)
    })?;
    let saved = match service.execute(Command::SaveConfig(config)).await? {
        CommandResult::Config(config) => *config,
        _ => unreachable!("SaveConfig returned an unexpected result"),
    };
    apply_tray_language(&app, saved.app.language).map_err(|error| {
        CommandError::new(
            print_bridge_cli::CommandErrorKind::Runtime,
            error.to_string(),
        )
    })?;

    Ok(saved)
}

/// 导出当前配置到加密文件。
#[tauri::command]
pub async fn export_config_file(
    path: String,
    password: String,
    options: ExportConfigOptions,
    service: State<'_, Arc<CommandService>>,
) -> Result<(), CommandError> {
    service
        .execute(Command::ExportConfig {
            path: PathBuf::from(path),
            password,
            options,
        })
        .await?;
    Ok(())
}

/// 解密配置文件并返回导入前差异预览。
#[tauri::command]
pub async fn preview_config_import(
    path: String,
    password: String,
    service: State<'_, Arc<CommandService>>,
) -> Result<ImportPreview, CommandError> {
    match service
        .execute(Command::PreviewConfigImport {
            path: PathBuf::from(path),
            password,
        })
        .await?
    {
        CommandResult::ImportPreview(preview) => Ok(preview),
        _ => unreachable!("PreviewConfigImport returned an unexpected result"),
    }
}

/// 解密并导入配置文件，要求文件哈希和预览时一致。
#[tauri::command]
pub async fn import_config_file(
    path: String,
    password: String,
    expected_file_hash: String,
    service: State<'_, Arc<CommandService>>,
) -> Result<AgentConfig, CommandError> {
    match service
        .execute(Command::ImportConfig {
            path: PathBuf::from(path),
            password,
            expected_file_hash,
        })
        .await?
    {
        CommandResult::Config(config) => Ok(*config),
        _ => unreachable!("ImportConfig returned an unexpected result"),
    }
}

/// 使用远程配置执行 GET/POST 连接测试。
#[tauri::command]
pub async fn test_remote_connection(
    config: AgentConfig,
    service: State<'_, Arc<CommandService>>,
) -> Result<(), CommandError> {
    service
        .execute(Command::TestRemoteConnection { config })
        .await?;
    Ok(())
}

/// 通过 AgentState 保存配置，便于测试绕过 Tauri app handle。
#[cfg(test)]
pub(crate) async fn save_config_for_state(
    config: AgentConfig,
    state: &AgentState,
) -> Result<AgentConfig, String> {
    state
        .save_config(config)
        .await
        .map_err(|error| error.to_string())
}

/// 通过 Tauri 自启动插件启用或禁用系统开机自启。
fn apply_autostart(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    }
    .map_err(|error| error.to_string())
}

fn check_service_port_available_for_host(host: &str, port: u16) -> Result<(), String> {
    if !(MIN_SERVICE_PORT..=MAX_SERVICE_PORT).contains(&port) {
        return Err(format!(
            "本地端口必须在 {MIN_SERVICE_PORT} 到 {MAX_SERVICE_PORT} 之间"
        ));
    }

    TcpListener::bind((host, port))
        .map(drop)
        .map_err(|_| format!("本地端口 {port} 已被占用，请换一个端口"))
}

/// 返回最近任务日志给 Tauri 前端。
#[tauri::command]
pub async fn get_logs(
    service: State<'_, Arc<CommandService>>,
) -> Result<Vec<TaskLogEntry>, CommandError> {
    match service.execute(Command::GetLogs).await? {
        CommandResult::Logs(logs) => Ok(logs),
        _ => unreachable!("GetLogs returned an unexpected result"),
    }
}

/// 返回本地任务历史摘要给 Tauri 前端。
#[tauri::command]
pub async fn get_task_history(
    service: State<'_, Arc<CommandService>>,
) -> Result<Vec<TaskHistoryJob>, CommandError> {
    match service.execute(Command::GetTaskHistory).await? {
        CommandResult::TaskHistory(jobs) => Ok(jobs),
        _ => unreachable!("GetTaskHistory returned an unexpected result"),
    }
}

/// 返回指定任务的历史状态事件给 Tauri 前端。
#[tauri::command]
pub async fn get_task_history_events(
    job_id: String,
    service: State<'_, Arc<CommandService>>,
) -> Result<Vec<TaskHistoryEvent>, CommandError> {
    match service
        .execute(Command::GetTaskHistoryEvents { job_id })
        .await?
    {
        CommandResult::TaskHistoryEvents(events) => Ok(events),
        _ => unreachable!("GetTaskHistoryEvents returned an unexpected result"),
    }
}

/// 清空本地任务日志和任务历史。
#[tauri::command]
pub async fn clear_task_history(
    service: State<'_, Arc<CommandService>>,
) -> Result<(), CommandError> {
    service.execute(Command::ClearTaskHistory).await?;
    Ok(())
}

/// 通过 AgentState 读取任务历史摘要，便于测试绕过 Tauri State。
#[cfg(test)]
pub(crate) fn get_task_history_for_state(
    state: &AgentState,
) -> Result<Vec<TaskHistoryJob>, String> {
    let Some(store) = &state.task_history else {
        return Ok(Vec::new());
    };
    store.recent_jobs(500).map_err(|error| error.to_string())
}

/// 通过 AgentState 读取指定任务事件，便于测试绕过 Tauri State。
#[cfg(test)]
pub(crate) fn get_task_history_events_for_state(
    job_id: &str,
    state: &AgentState,
) -> Result<Vec<TaskHistoryEvent>, String> {
    let Some(store) = &state.task_history else {
        return Ok(Vec::new());
    };
    store
        .events_for_job(job_id)
        .map_err(|error| error.to_string())
}

/// 通过 AgentState 清空任务历史，便于测试绕过 Tauri State。
#[cfg(test)]
pub(crate) async fn clear_task_history_for_state(state: &AgentState) -> Result<(), String> {
    state.logs.lock().await.clear();

    let Some(store) = &state.task_history else {
        return Ok(());
    };
    store.clear().map_err(|error| error.to_string())
}

/// 使用当前 Agent 默认打印设置提交一张校准测试页。
#[tauri::command]
pub async fn print_test(
    config: AgentConfig,
    service: State<'_, Arc<CommandService>>,
) -> Result<(), CommandError> {
    service.execute(Command::TestPrint { config }).await?;
    Ok(())
}

/// 返回系统打印机列表。
#[tauri::command]
pub async fn list_printers(
    service: State<'_, Arc<CommandService>>,
) -> Result<Vec<PrinterInfo>, CommandError> {
    match service.execute(Command::ListPrinters).await? {
        CommandResult::Printers(printers) => Ok(printers),
        _ => unreachable!("ListPrinters returned an unexpected result"),
    }
}

/// 返回指定打印机支持的纸张。
#[tauri::command]
pub async fn list_papers(
    printer_name: String,
    service: State<'_, Arc<CommandService>>,
) -> Result<Vec<PaperInfo>, CommandError> {
    match service
        .execute(Command::ListPapers { printer_name })
        .await?
    {
        CommandResult::Papers(papers) => Ok(papers),
        _ => unreachable!("ListPapers returned an unexpected result"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        check_service_port_available_for_host, clear_task_history_for_state,
        get_task_history_events_for_state, get_task_history_for_state, save_config_for_state,
        MAX_SERVICE_PORT, MIN_SERVICE_PORT,
    };
    use crate::{
        config::AgentConfig,
        logs::TaskLogEntry,
        protocol::JobStatus,
        state::AgentState,
        task_history::{
            NewTaskHistoryEvent, TaskHistorySource, TaskHistoryStatus, TaskHistoryStore,
        },
    };
    use std::fs;

    #[tokio::test]
    async fn save_config_persists_to_state_config_path() {
        let path = std::env::temp_dir().join(format!(
            "print-bridge-command-save-config-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let state = AgentState::with_config_path(AgentConfig::default(), path.clone());
        let mut config = AgentConfig::default();
        config.service.port = 19090;

        save_config_for_state(config.clone(), &state).await.unwrap();

        let loaded = AgentConfig::load(&path).unwrap();
        assert_eq!(loaded, config);

        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn save_config_for_state_normalizes_service_host_before_persisting() {
        let path = std::env::temp_dir().join(format!(
            "print-bridge-command-save-config-normalized-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let state = AgentState::with_config_path(AgentConfig::default(), path.clone());
        let mut config = AgentConfig::default();
        config.service.host = "0.0.0.0".to_string();
        config.service.port = 19091;

        let saved = save_config_for_state(config, &state).await.unwrap();

        assert_eq!(saved.service.host, "127.0.0.1");
        assert_eq!(state.config.read().await.service.host, "127.0.0.1");
        let loaded = AgentConfig::load(&path).unwrap();
        assert_eq!(loaded.service.host, "127.0.0.1");
        assert_eq!(loaded.service.port, 19091);

        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn save_config_for_state_rejects_invalid_allowed_origins() {
        let path = std::env::temp_dir().join(format!(
            "print-bridge-command-save-config-invalid-origin-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let state = AgentState::with_config_path(AgentConfig::default(), path.clone());
        let mut config = AgentConfig::default();
        config.security.allowed_origins = vec!["not-a-url".to_string()];

        let error = save_config_for_state(config, &state).await.unwrap_err();

        assert_eq!(error, "invalid origin");
        assert!(!path.exists());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn check_service_port_available_rejects_occupied_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let error = check_service_port_available_for_host("127.0.0.1", port).unwrap_err();

        assert_eq!(error, format!("本地端口 {port} 已被占用，请换一个端口"));
    }

    #[test]
    fn check_service_port_available_rejects_ports_below_minimum() {
        let error =
            check_service_port_available_for_host("127.0.0.1", MIN_SERVICE_PORT - 1).unwrap_err();

        assert_eq!(
            error,
            format!("本地端口必须在 {MIN_SERVICE_PORT} 到 {MAX_SERVICE_PORT} 之间")
        );
    }

    #[tokio::test]
    async fn task_history_commands_return_empty_without_store() {
        let state = AgentState::new(AgentConfig::default());

        assert!(get_task_history_for_state(&state).unwrap().is_empty());
        assert!(get_task_history_events_for_state("job-1", &state)
            .unwrap()
            .is_empty());
        clear_task_history_for_state(&state).await.unwrap();
    }

    #[tokio::test]
    async fn task_history_commands_read_events_and_clear_store() {
        let store = TaskHistoryStore::open_in_memory().unwrap();
        store
            .record_event(&NewTaskHistoryEvent {
                job_id: "job-1",
                request_id: Some("request-1"),
                batch_id: None,
                source: TaskHistorySource::Test,
                status: TaskHistoryStatus::Queued,
                message: Some("queued"),
                printer_name: Some("Office Printer"),
                paper_name: Some("A4"),
                copies: Some(1),
                occurred_at: "2026-07-06T10:00:00Z",
            })
            .unwrap();
        let state = AgentState::new(AgentConfig::default()).with_task_history_store(store);

        let jobs = get_task_history_for_state(&state).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "job-1");
        assert_eq!(jobs[0].source, TaskHistorySource::Test);

        let events = get_task_history_events_for_state("job-1", &state).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, TaskHistoryStatus::Queued);

        clear_task_history_for_state(&state).await.unwrap();
        assert!(get_task_history_for_state(&state).unwrap().is_empty());
        assert!(get_task_history_events_for_state("job-1", &state)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn clear_task_history_for_state_clears_recent_logs() {
        let state = AgentState::new(AgentConfig::default());
        state.logs.lock().await.push(TaskLogEntry {
            timestamp: "2026-07-06T10:00:00Z".to_string(),
            request_id: Some("request-1".to_string()),
            batch_id: None,
            job_id: Some("job-1".to_string()),
            origin: Some("http://localhost:5173".to_string()),
            status: JobStatus::Queued,
            message: "queued".to_string(),
        });

        clear_task_history_for_state(&state).await.unwrap();

        assert!(state.logs.lock().await.recent().is_empty());
    }
}
