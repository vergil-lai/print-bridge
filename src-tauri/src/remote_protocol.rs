use crate::protocol::PrintJobInput;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

#[derive(Debug, Error)]
pub enum RemoteProtocolError {
    #[error("invalid remote task response")]
    InvalidResponse(#[from] serde_json::Error),
}

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
