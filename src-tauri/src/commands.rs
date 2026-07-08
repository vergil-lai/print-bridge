use crate::{
    app_state::AppState,
    config::{AgentConfig, MAX_SERVICE_PORT, MIN_SERVICE_PORT},
    config_transfer::{
        build_transfer_payload, decrypt_payload, encrypt_payload, merge_payload, preview_payload,
        read_encrypted_file_with_hash, write_encrypted_file, ExportConfigOptions, ImportPreview,
    },
    logs::TaskLogEntry,
    remote_client::RemoteClient,
    task_history::{TaskHistoryEvent, TaskHistoryJob},
    test_print::print_calibration_page_with_config,
};
use std::{net::TcpListener, path::PathBuf};
use tauri::State;
use tauri_plugin_autostart::ManagerExt;

/// 返回当前 Agent 配置给 Tauri 前端。
#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<AgentConfig, String> {
    Ok(state.config.read().await.clone())
}

/// 返回当前运行的桌面应用是否为 debug 构建。
#[tauri::command]
pub fn is_debug_build() -> bool {
    cfg!(debug_assertions)
}

/// 应用应用层设置，并保存完整 Agent 配置。
#[tauri::command]
pub async fn save_config(
    app: tauri::AppHandle,
    config: AgentConfig,
    state: State<'_, AppState>,
) -> Result<AgentConfig, String> {
    let current_port = state.config.read().await.service.port;
    if config.service.port != current_port {
        check_service_port_available_for_host("0.0.0.0", config.service.port)?;
    }

    apply_autostart(&app, config.app.autostart)?;
    save_config_for_state(config, &state).await
}

/// 导出当前配置到加密文件。
#[tauri::command]
pub async fn export_config_file(
    path: String,
    password: String,
    options: ExportConfigOptions,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let current = state.config.read().await.clone();
    let payload = build_transfer_payload(&current, &options);
    let encrypted = encrypt_payload(&payload, &password).map_err(|error| error.to_string())?;
    write_encrypted_file(&PathBuf::from(path), &encrypted).map_err(|error| error.to_string())
}

/// 解密配置文件并返回导入前差异预览。
#[tauri::command]
pub async fn preview_config_import(
    path: String,
    password: String,
    state: State<'_, AppState>,
) -> Result<ImportPreview, String> {
    let current = state.config.read().await.clone();
    let (encrypted, file_hash) =
        read_encrypted_file_with_hash(&PathBuf::from(path)).map_err(|error| error.to_string())?;
    let payload = decrypt_payload(&encrypted, &password).map_err(|error| error.to_string())?;
    let mut preview = preview_payload(&current, &payload).map_err(|error| error.to_string())?;
    preview.file_hash = file_hash;
    Ok(preview)
}

/// 解密并导入配置文件，要求文件哈希和预览时一致。
#[tauri::command]
pub async fn import_config_file(
    path: String,
    password: String,
    expected_file_hash: String,
    state: State<'_, AppState>,
) -> Result<AgentConfig, String> {
    let current = state.config.read().await.clone();
    let (encrypted, file_hash) =
        read_encrypted_file_with_hash(&PathBuf::from(path)).map_err(|error| error.to_string())?;
    if file_hash != expected_file_hash {
        return Err("配置文件已变化，请重新预览后导入".to_string());
    }
    let payload = decrypt_payload(&encrypted, &password).map_err(|error| error.to_string())?;
    let merged = merge_payload(&current, &payload).map_err(|error| error.to_string())?;
    save_config_for_state(merged, &state).await
}

/// 使用远程配置执行 GET/POST 连接测试。
#[tauri::command]
pub async fn test_remote_connection(config: AgentConfig) -> Result<(), String> {
    if !config.remote.enabled {
        return Ok(());
    }

    RemoteClient::default()
        .test_connection(&config.remote)
        .await
        .map_err(|error| error.to_string())
}

/// 通过 AppState 保存配置，便于测试绕过 Tauri app handle。
pub(crate) async fn save_config_for_state(
    config: AgentConfig,
    state: &AppState,
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
pub async fn get_logs(state: State<'_, AppState>) -> Result<Vec<TaskLogEntry>, String> {
    Ok(state.logs.lock().await.recent())
}

/// 返回本地任务历史摘要给 Tauri 前端。
#[tauri::command]
pub async fn get_task_history(state: State<'_, AppState>) -> Result<Vec<TaskHistoryJob>, String> {
    get_task_history_for_state(&state)
}

/// 返回指定任务的历史状态事件给 Tauri 前端。
#[tauri::command]
pub async fn get_task_history_events(
    job_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<TaskHistoryEvent>, String> {
    get_task_history_events_for_state(&job_id, &state)
}

/// 清空本地任务日志和任务历史。
#[tauri::command]
pub async fn clear_task_history(state: State<'_, AppState>) -> Result<(), String> {
    clear_task_history_for_state(&state).await
}

/// 通过 AppState 读取任务历史摘要，便于测试绕过 Tauri State。
pub(crate) fn get_task_history_for_state(state: &AppState) -> Result<Vec<TaskHistoryJob>, String> {
    let Some(store) = &state.task_history else {
        return Ok(Vec::new());
    };
    store.recent_jobs(500).map_err(|error| error.to_string())
}

/// 通过 AppState 读取指定任务事件，便于测试绕过 Tauri State。
pub(crate) fn get_task_history_events_for_state(
    job_id: &str,
    state: &AppState,
) -> Result<Vec<TaskHistoryEvent>, String> {
    let Some(store) = &state.task_history else {
        return Ok(Vec::new());
    };
    store
        .events_for_job(job_id)
        .map_err(|error| error.to_string())
}

/// 通过 AppState 清空任务历史，便于测试绕过 Tauri State。
pub(crate) async fn clear_task_history_for_state(state: &AppState) -> Result<(), String> {
    state.logs.lock().await.clear();

    let Some(store) = &state.task_history else {
        return Ok(());
    };
    store.clear().map_err(|error| error.to_string())
}

/// 使用当前 Agent 默认打印设置提交一张校准测试页。
#[tauri::command]
pub async fn print_test(config: AgentConfig, state: State<'_, AppState>) -> Result<(), String> {
    print_calibration_page_with_config(&state, config)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        check_service_port_available_for_host, clear_task_history_for_state,
        get_task_history_events_for_state, get_task_history_for_state, save_config_for_state,
        MAX_SERVICE_PORT, MIN_SERVICE_PORT,
    };
    use crate::{
        app_state::AppState,
        config::AgentConfig,
        logs::TaskLogEntry,
        protocol::JobStatus,
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
        let state = AppState::with_config_path(AgentConfig::default(), path.clone());
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
        let state = AppState::with_config_path(AgentConfig::default(), path.clone());
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
        let state = AppState::with_config_path(AgentConfig::default(), path.clone());
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
        let state = AppState::new(AgentConfig::default());

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
        let state = AppState::new(AgentConfig::default()).with_task_history_store(store);

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
        let state = AppState::new(AgentConfig::default());
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
