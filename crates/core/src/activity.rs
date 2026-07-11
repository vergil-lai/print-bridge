use serde::{Deserialize, Serialize};

use crate::protocol::JobStatus;

/// 一条本地保存并通过 WebSocket 广播的任务状态记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLogEntry {
    pub timestamp: String,
    pub request_id: Option<String>,
    pub batch_id: Option<String>,
    pub job_id: Option<String>,
    pub origin: Option<String>,
    pub status: JobStatus,
    pub message: String,
}

/// 本地任务历史记录中的任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskHistoryStatus {
    Queued,
    Downloading,
    Printing,
    Submitted,
    Completed,
    Failed,
    Unknown,
    Cancelled,
}

impl TaskHistoryStatus {
    /// 返回持久化时使用的状态字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Printing => "printing",
            Self::Submitted => "submitted",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
        }
    }

    /// 从持久化字符串恢复状态。
    pub fn from_storage_str(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "downloading" => Some(Self::Downloading),
            "printing" => Some(Self::Printing),
            "submitted" => Some(Self::Submitted),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "unknown" => Some(Self::Unknown),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// 返回该状态是否结束任务生命周期。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Submitted | Self::Completed | Self::Failed | Self::Unknown | Self::Cancelled
        )
    }
}

/// 本地任务历史记录中的任务来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskHistorySource {
    WebSocket,
    Remote,
    Test,
}

impl TaskHistorySource {
    /// 返回持久化时使用的来源字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WebSocket => "web_socket",
            Self::Remote => "remote",
            Self::Test => "test",
        }
    }

    /// 从持久化字符串恢复来源。
    pub fn from_storage_str(value: &str) -> Option<Self> {
        match value {
            "web_socket" => Some(Self::WebSocket),
            "remote" => Some(Self::Remote),
            "test" => Some(Self::Test),
            _ => None,
        }
    }
}

/// 任务历史列表中的单个任务摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryJob {
    pub job_id: String,
    pub request_id: Option<String>,
    pub batch_id: Option<String>,
    pub source: TaskHistorySource,
    pub current_status: TaskHistoryStatus,
    pub current_message: Option<String>,
    pub printer_name: Option<String>,
    pub paper_name: Option<String>,
    pub copies: Option<u16>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

/// 任务历史中的单个状态事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistoryEvent {
    pub id: i64,
    pub job_id: String,
    pub status: TaskHistoryStatus,
    pub message: Option<String>,
    pub occurred_at: String,
}
