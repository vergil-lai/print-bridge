use crate::protocol::{validate_html_file_url, PrintJobInput, SupportedFormat};
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
    #[error("invalid remote task: {0}")]
    InvalidMessage(String),
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
        let tasks: Vec<RemoteTask> =
            serde_json::from_value(value).map_err(RemoteProtocolError::InvalidResponse)?;
        validate_html_sources(&tasks)?;
        return Ok(tasks);
    }

    let task = serde_json::from_value(value)?;
    validate_html_sources(std::slice::from_ref(&task))?;
    Ok(vec![task])
}

/// 在远程任务入队前拒绝无法被安全 HTML 渲染器加载的 URL。
fn validate_html_sources(tasks: &[RemoteTask]) -> Result<(), RemoteProtocolError> {
    for task in tasks {
        let jobs: &[PrintJobInput] = match task {
            RemoteTask::Print { job, .. } => std::slice::from_ref(job),
            RemoteTask::PrintBatch { jobs, .. } => jobs,
        };

        for job in jobs {
            if job.format != SupportedFormat::Html {
                continue;
            }
            let file_url = job
                .file_url
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    RemoteProtocolError::InvalidMessage("html job requires file_url".to_string())
                })?;
            validate_html_file_url(file_url)
                .map_err(|error| RemoteProtocolError::InvalidMessage(error.to_string()))?;
        }
    }

    Ok(())
}
