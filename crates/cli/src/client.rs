use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::{Command, CommandError, CommandErrorKind, CommandExecutor, CommandResult};
use async_trait::async_trait;

pub const IPC_PROTOCOL_VERSION: u16 = 1;
pub const MAX_IPC_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// 本地 IPC 命令请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRequest {
    pub protocol_version: u16,
    pub request_id: Uuid,
    pub command: Command,
}

/// 本地 IPC 命令响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    pub protocol_version: u16,
    pub request_id: Uuid,
    pub result: Result<CommandResult, CommandError>,
}

/// 通过本机 socket 或命名管道调用运行中的 Agent。
pub struct LocalCommandClient;

/// 把本地 IPC client 适配为 CommandService 的在线 executor。
pub struct LocalClientExecutor {
    path: PathBuf,
}

impl LocalClientExecutor {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[async_trait]
impl CommandExecutor for LocalClientExecutor {
    async fn execute(&self, command: Command) -> Result<CommandResult, CommandError> {
        LocalCommandClient::execute(&self.path, command).await
    }
}

impl LocalCommandClient {
    /// 向 Unix socket 发送单个命令。
    #[cfg(unix)]
    pub async fn execute(path: &Path, command: Command) -> Result<CommandResult, CommandError> {
        let mut stream = match tokio::net::UnixStream::connect(path).await {
            Ok(stream) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(CommandError::new(
                    CommandErrorKind::NotRunning,
                    "agent is not running",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(CommandError::new(
                    CommandErrorKind::PermissionDenied,
                    error.to_string(),
                ));
            }
            Err(error) => {
                return Err(CommandError::new(
                    CommandErrorKind::Runtime,
                    error.to_string(),
                ));
            }
        };
        exchange(&mut stream, command).await
    }

    /// 通过 Windows 命名管道发送单个命令。
    #[cfg(windows)]
    pub async fn execute(path: &Path, command: Command) -> Result<CommandResult, CommandError> {
        use tokio::net::windows::named_pipe::ClientOptions;

        let name = path.to_string_lossy();
        let mut stream = ClientOptions::new().open(name.as_ref()).map_err(|error| {
            let kind = if error.kind() == std::io::ErrorKind::NotFound {
                CommandErrorKind::NotRunning
            } else if error.kind() == std::io::ErrorKind::PermissionDenied {
                CommandErrorKind::PermissionDenied
            } else {
                CommandErrorKind::Runtime
            };
            CommandError::new(kind, error.to_string())
        })?;
        exchange(&mut stream, command).await
    }
}

async fn exchange<S>(stream: &mut S, command: Command) -> Result<CommandResult, CommandError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let request = CommandRequest {
        protocol_version: IPC_PROTOCOL_VERSION,
        request_id: Uuid::new_v4(),
        command,
    };
    let payload = serde_json::to_vec(&request)
        .map_err(|error| CommandError::new(CommandErrorKind::Runtime, error.to_string()))?;
    write_frame(stream, &payload).await?;
    let response_payload = tokio::time::timeout(Duration::from_secs(10), read_frame(stream))
        .await
        .map_err(|_| CommandError::new(CommandErrorKind::Runtime, "IPC request timed out"))??;
    let response: CommandResponse = serde_json::from_slice(&response_payload)
        .map_err(|error| CommandError::new(CommandErrorKind::Runtime, error.to_string()))?;
    if response.protocol_version != IPC_PROTOCOL_VERSION
        || response.request_id != request.request_id
    {
        return Err(CommandError::new(
            CommandErrorKind::Runtime,
            "invalid IPC response envelope",
        ));
    }
    response.result
}

/// 写入长度前缀帧。
pub async fn write_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<(), CommandError> {
    if payload.len() > MAX_IPC_FRAME_BYTES {
        return Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "IPC frame is too large",
        ));
    }
    writer
        .write_u32(payload.len() as u32)
        .await
        .map_err(|error| CommandError::new(CommandErrorKind::Runtime, error.to_string()))?;
    writer
        .write_all(payload)
        .await
        .map_err(|error| CommandError::new(CommandErrorKind::Runtime, error.to_string()))
}

/// 读取长度前缀帧。
pub async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, CommandError> {
    let len = reader
        .read_u32()
        .await
        .map_err(|error| CommandError::new(CommandErrorKind::Runtime, error.to_string()))?
        as usize;
    if len > MAX_IPC_FRAME_BYTES {
        return Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "IPC frame is too large",
        ));
    }
    let mut payload = vec![0; len];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|error| CommandError::new(CommandErrorKind::Runtime, error.to_string()))?;
    Ok(payload)
}
