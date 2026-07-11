use std::net::SocketAddr;

use print_bridge_core::{
    activity::{TaskHistoryEvent, TaskHistoryJob, TaskLogEntry},
    config::AgentConfig,
    printing::{PaperInfo, PrinterInfo},
};
use serde::{Deserialize, Serialize};

use crate::config_transfer::ImportPreview;

/// 命令失败的稳定分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandErrorKind {
    InvalidInput,
    NotRunning,
    PermissionDenied,
    Conflict,
    Runtime,
}

/// 可跨 CLI 与 IPC 边界传递的命令错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub message: String,
}

impl CommandError {
    /// 创建带稳定分类的命令错误。
    pub fn new(kind: CommandErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

/// Agent 运行状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub running: bool,
    pub listen_addr: Option<SocketAddr>,
}

/// 功能命令的结构化结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", content = "payload", rename_all = "snake_case")]
pub enum CommandResult {
    Empty,
    Config(Box<AgentConfig>),
    Printers(Vec<PrinterInfo>),
    Papers(Vec<PaperInfo>),
    Logs(Vec<TaskLogEntry>),
    TaskHistory(Vec<TaskHistoryJob>),
    TaskHistoryEvents(Vec<TaskHistoryEvent>),
    ImportPreview(ImportPreview),
    Status(AgentStatus),
}
