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
    Unsupported,
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

    /// 返回功能 CLI 使用的稳定进程退出码。
    pub fn exit_code(&self) -> i32 {
        match self.kind {
            CommandErrorKind::InvalidInput => 2,
            CommandErrorKind::NotRunning => 3,
            CommandErrorKind::PermissionDenied => 4,
            CommandErrorKind::Conflict => 5,
            CommandErrorKind::Runtime | CommandErrorKind::Unsupported => 1,
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

/// 提供 CLI 的产品类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductKind {
    Desktop,
    Headless,
}

/// 单项 Doctor 检查状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

/// Doctor 的单项只读检查结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub code: String,
    pub status: DoctorStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Doctor 汇总计数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorSummary {
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
}

/// Doctor 的稳定结构化报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
    pub summary: DoctorSummary,
}

impl DoctorReport {
    /// 从检查项计算汇总。
    pub fn new(checks: Vec<DoctorCheck>) -> Self {
        let mut summary = DoctorSummary {
            pass: 0,
            warn: 0,
            fail: 0,
        };
        for check in &checks {
            match check.status {
                DoctorStatus::Pass => summary.pass += 1,
                DoctorStatus::Warn => summary.warn += 1,
                DoctorStatus::Fail => summary.fail += 1,
            }
        }
        Self { checks, summary }
    }
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
    Doctor(DoctorReport),
    Status(AgentStatus),
}
