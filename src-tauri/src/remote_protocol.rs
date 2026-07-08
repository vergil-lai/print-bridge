use crate::protocol::PrintJobInput;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 远程任务服务下发给 Agent 的任务消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RemoteTask {
    #[serde(rename = "print")]
    Print {
        request_id: String,
        #[serde(flatten)]
        job: PrintJobInput,
    },
    #[serde(rename = "print_batch")]
    PrintBatch {
        request_id: String,
        batch_id: String,
        jobs: Vec<PrintJobInput>,
    },
}

/// 远程任务响应无法解析时返回的错误。
#[derive(Debug, Error)]
pub enum RemoteProtocolError {
    #[error("invalid remote task response")]
    InvalidResponse(#[from] serde_json::Error),
}

/// 解析远程任务响应，兼容单个任务、数组、空响应和 null。
pub fn parse_remote_tasks(input: &str) -> Result<Vec<RemoteTask>, RemoteProtocolError> {
    if input.trim().is_empty() {
        return Ok(Vec::new());
    }

    let value: serde_json::Value = serde_json::from_str(input)?;
    if value.is_null() {
        return Ok(Vec::new());
    }

    if value.is_array() {
        return serde_json::from_value(value).map_err(RemoteProtocolError::InvalidResponse);
    }

    let task = serde_json::from_value(value)?;
    Ok(vec![task])
}
