use std::{
    io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use print_bridge_cli::{
    client::{read_frame, write_frame, CommandRequest, CommandResponse, IPC_PROTOCOL_VERSION},
    CommandError, CommandErrorKind, CommandExecutor,
};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 返回 runtime 目录内稳定的 Agent socket 路径。
pub fn socket_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("agent.sock")
}

/// 监听本地 Unix socket，直到 runtime 发出取消信号。
pub async fn serve_until(
    path: PathBuf,
    executor: Arc<dyn CommandExecutor>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let executor = executor.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(stream, executor).await;
                });
            }
        }
    }

    drop(listener);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// 同步完成 socket 绑定后再启动后台服务，确保启动错误可直接返回。
pub fn bind_and_spawn(
    path: PathBuf,
    executor: Arc<dyn CommandExecutor>,
    shutdown: CancellationToken,
) -> io::Result<JoinHandle<io::Result<()>>> {
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
    Ok(tokio::spawn(serve_listener_until(
        listener, path, executor, shutdown,
    )))
}

async fn serve_listener_until(
    listener: UnixListener,
    path: PathBuf,
    executor: Arc<dyn CommandExecutor>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let executor = executor.clone();
                tokio::spawn(async move { let _ = handle_connection(stream, executor).await; });
            }
        }
    }
    drop(listener);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

async fn handle_connection(
    mut stream: UnixStream,
    executor: Arc<dyn CommandExecutor>,
) -> Result<(), CommandError> {
    let payload = read_frame(&mut stream).await?;
    let request: CommandRequest = serde_json::from_slice(&payload)
        .map_err(|error| CommandError::new(CommandErrorKind::InvalidInput, error.to_string()))?;
    let result = if request.protocol_version == IPC_PROTOCOL_VERSION {
        executor.execute(request.command).await
    } else {
        Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "unsupported IPC protocol version",
        ))
    };
    let response = CommandResponse {
        protocol_version: IPC_PROTOCOL_VERSION,
        request_id: request.request_id,
        result,
    };
    let payload = serde_json::to_vec(&response)
        .map_err(|error| CommandError::new(CommandErrorKind::Runtime, error.to_string()))?;
    write_frame(&mut stream, &payload).await
}
