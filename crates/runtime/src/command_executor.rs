use std::net::SocketAddr;

use async_trait::async_trait;
use print_bridge_cli::{
    config_transfer::{
        build_transfer_payload, decrypt_payload, encrypt_payload, merge_payload, preview_payload,
        read_encrypted_file_with_hash, write_encrypted_file,
    },
    AgentStatus, Command, CommandError, CommandErrorKind, CommandExecutor, CommandResult,
};

use crate::{
    config::AgentConfig, remote_client::RemoteClient, state::AgentState,
    test_print::print_calibration_page_with_config,
};

/// 使用运行中 AgentState 执行共享功能命令。
pub struct RuntimeCommandExecutor {
    state: AgentState,
    listen_addr: SocketAddr,
}

impl RuntimeCommandExecutor {
    /// 创建运行时命令执行器。
    pub fn new(state: AgentState, listen_addr: SocketAddr) -> Self {
        Self { state, listen_addr }
    }

    fn runtime_error(error: impl ToString) -> CommandError {
        CommandError::new(CommandErrorKind::Runtime, error.to_string())
    }
}

#[async_trait]
impl CommandExecutor for RuntimeCommandExecutor {
    async fn execute(&self, command: Command) -> Result<CommandResult, CommandError> {
        match command {
            Command::GetConfig => Ok(CommandResult::Config(Box::new(
                self.state.config.read().await.clone(),
            ))),
            Command::SaveConfig(config) => self
                .state
                .save_config(config)
                .await
                .map(|config| CommandResult::Config(Box::new(config)))
                .map_err(Self::runtime_error),
            Command::ListPrinters => self
                .state
                .printing
                .list_printers()
                .map(CommandResult::Printers)
                .map_err(Self::runtime_error),
            Command::ListPapers { printer_name } => self
                .state
                .printing
                .list_papers(&printer_name)
                .map(CommandResult::Papers)
                .map_err(Self::runtime_error),
            Command::GetLogs => Ok(CommandResult::Logs(self.state.logs.lock().await.recent())),
            Command::GetTaskHistory => {
                let jobs = self
                    .state
                    .task_history
                    .as_ref()
                    .map(|store| store.recent_jobs(500))
                    .transpose()
                    .map_err(Self::runtime_error)?
                    .unwrap_or_default();
                Ok(CommandResult::TaskHistory(jobs))
            }
            Command::GetTaskHistoryEvents { job_id } => {
                let events = self
                    .state
                    .task_history
                    .as_ref()
                    .map(|store| store.events_for_job(&job_id))
                    .transpose()
                    .map_err(Self::runtime_error)?
                    .unwrap_or_default();
                Ok(CommandResult::TaskHistoryEvents(events))
            }
            Command::ClearTaskHistory => {
                self.state.logs.lock().await.clear();
                if let Some(store) = &self.state.task_history {
                    store.clear().map_err(Self::runtime_error)?;
                }
                Ok(CommandResult::Empty)
            }
            Command::ExportConfig {
                path,
                password,
                options,
            } => {
                let current = self.state.config.read().await.clone();
                let payload = build_transfer_payload(&current, &options);
                let encrypted =
                    encrypt_payload(&payload, &password).map_err(Self::runtime_error)?;
                write_encrypted_file(&path, &encrypted).map_err(Self::runtime_error)?;
                Ok(CommandResult::Empty)
            }
            Command::PreviewConfigImport { path, password } => {
                let current = self.state.config.read().await.clone();
                let (encrypted, file_hash) =
                    read_encrypted_file_with_hash(&path).map_err(Self::runtime_error)?;
                let payload =
                    decrypt_payload(&encrypted, &password).map_err(Self::runtime_error)?;
                let mut preview =
                    preview_payload(&current, &payload).map_err(Self::runtime_error)?;
                preview.file_hash = file_hash;
                Ok(CommandResult::ImportPreview(preview))
            }
            Command::ImportConfig {
                path,
                password,
                expected_file_hash,
            } => {
                let current = self.state.config.read().await.clone();
                let (encrypted, file_hash) =
                    read_encrypted_file_with_hash(&path).map_err(Self::runtime_error)?;
                if file_hash != expected_file_hash {
                    return Err(CommandError::new(
                        CommandErrorKind::Conflict,
                        "配置文件已变化，请重新预览后导入",
                    ));
                }
                let payload =
                    decrypt_payload(&encrypted, &password).map_err(Self::runtime_error)?;
                let merged = merge_payload(&current, &payload).map_err(Self::runtime_error)?;
                self.state
                    .save_config(merged)
                    .await
                    .map(|config| CommandResult::Config(Box::new(config)))
                    .map_err(Self::runtime_error)
            }
            Command::TestRemoteConnection { config } => {
                if config.remote.enabled {
                    RemoteClient::default()
                        .test_connection(&config.remote)
                        .await
                        .map_err(Self::runtime_error)?;
                }
                Ok(CommandResult::Empty)
            }
            Command::TestPrint { config } => {
                print_calibration_page_with_config(&self.state, config)
                    .await
                    .map_err(Self::runtime_error)?;
                Ok(CommandResult::Empty)
            }
            Command::ValidateConfig { path } => {
                let path = path
                    .or_else(|| self.state.config_path.clone())
                    .ok_or_else(|| {
                        CommandError::new(CommandErrorKind::InvalidInput, "config path is required")
                    })?;
                AgentConfig::load(&path).map_err(Self::runtime_error)?;
                Ok(CommandResult::Empty)
            }
            Command::Status => Ok(CommandResult::Status(AgentStatus {
                running: true,
                listen_addr: Some(self.listen_addr),
            })),
        }
    }
}
