use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use print_bridge_cli::{
    client::{read_frame, write_frame, CommandRequest, CommandResponse, IPC_PROTOCOL_VERSION},
    CommandError, CommandErrorKind, CommandExecutor,
};
use tokio::{
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

/// 返回 Windows 命名管道的稳定标识。
pub fn socket_path(_runtime_dir: &Path) -> PathBuf {
    PathBuf::from(r"\\.\pipe\print-bridge-agent")
}

/// 启动 Windows 命名管道后台服务。
pub fn bind_and_spawn(
    path: PathBuf,
    executor: Arc<dyn CommandExecutor>,
    shutdown: CancellationToken,
) -> io::Result<JoinHandle<io::Result<()>>> {
    let name = path.to_string_lossy().into_owned();
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    Ok(tokio::spawn(serve_named_pipe(
        first, name, executor, shutdown,
    )))
}

/// 直接运行 Windows 命名管道服务。
pub async fn serve_until(
    path: PathBuf,
    executor: Arc<dyn CommandExecutor>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let name = path.to_string_lossy().into_owned();
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    serve_named_pipe(first, name, executor, shutdown).await
}

async fn serve_named_pipe(
    mut server: NamedPipeServer,
    name: String,
    executor: Arc<dyn CommandExecutor>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            connected = server.connect() => connected?,
        }
        let next = ServerOptions::new().create(&name)?;
        let current = std::mem::replace(&mut server, next);
        let executor = executor.clone();
        tokio::spawn(async move {
            let _ = handle_connection(current, executor).await;
        });
    }
    Ok(())
}

async fn handle_connection(
    mut stream: NamedPipeServer,
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
