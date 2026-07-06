use crate::{
    app_state::AppState,
    config::AgentConfig,
    logs::TaskLogEntry,
    remote_client::RemoteClient,
    task_history::{TaskHistoryEvent, TaskHistoryJob},
    test_print::print_calibration_page_with_config,
};
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
    apply_autostart(&app, config.app.autostart)?;
    save_config_for_state(config, &state).await
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

/// 返回最近任务日志给 Tauri 前端。
#[tauri::command]
pub async fn get_logs(state: State<'_, AppState>) -> Result<Vec<TaskLogEntry>, String> {
    Ok(state.logs.lock().await.recent())
}

#[tauri::command]
pub async fn get_task_history(state: State<'_, AppState>) -> Result<Vec<TaskHistoryJob>, String> {
    get_task_history_for_state(&state)
}

#[tauri::command]
pub async fn get_task_history_events(
    job_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<TaskHistoryEvent>, String> {
    get_task_history_events_for_state(&job_id, &state)
}

#[tauri::command]
pub async fn clear_task_history(state: State<'_, AppState>) -> Result<(), String> {
    clear_task_history_for_state(&state)
}

pub(crate) fn get_task_history_for_state(state: &AppState) -> Result<Vec<TaskHistoryJob>, String> {
    let Some(store) = &state.task_history else {
        return Ok(Vec::new());
    };
    store.recent_jobs(500).map_err(|error| error.to_string())
}

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

pub(crate) fn clear_task_history_for_state(state: &AppState) -> Result<(), String> {
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
        clear_task_history_for_state, get_task_history_events_for_state,
        get_task_history_for_state, save_config_for_state,
    };
    use crate::{
        app_state::AppState,
        config::AgentConfig,
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
    fn task_history_commands_return_empty_without_store() {
        let state = AppState::new(AgentConfig::default());

        assert!(get_task_history_for_state(&state).unwrap().is_empty());
        assert!(get_task_history_events_for_state("job-1", &state)
            .unwrap()
            .is_empty());
        clear_task_history_for_state(&state).unwrap();
    }

    #[test]
    fn task_history_commands_read_events_and_clear_store() {
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

        clear_task_history_for_state(&state).unwrap();
        assert!(get_task_history_for_state(&state).unwrap().is_empty());
        assert!(get_task_history_events_for_state("job-1", &state)
            .unwrap()
            .is_empty());
    }
}
