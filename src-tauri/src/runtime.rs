use crate::{
    agent_guard::{already_running_message, check_agent_port, AgentPortStatus, RunningAgent},
    app_state::AppState,
    config::{
        cli_config_path, cli_data_dir, cli_remote_store_path, cli_task_history_path, AgentConfig,
    },
    printing, queue, remote_store, remote_worker, server, task_history,
};
use std::{io, net::SocketAddr, path::PathBuf};
use thiserror::Error;

/// Headless serve 启动后可输出给终端的运行信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadlessRuntimeInfo {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub listen_addr: SocketAddr,
}

/// Headless runtime 启动或运行失败。
#[derive(Debug, Error)]
pub enum HeadlessRuntimeError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Server(#[from] server::ServerError),
    #[error(transparent)]
    RemoteStore(#[from] rusqlite::Error),
    #[error("server task failed: {0}")]
    ServerTaskJoin(#[from] tokio::task::JoinError),
    #[error("{}", already_running_message(.0))]
    AlreadyRunning(RunningAgent),
}

/// 从当前环境变量解析路径并运行 headless Agent。
pub fn run_headless_from_env() -> Result<HeadlessRuntimeInfo, HeadlessRuntimeError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_headless())
}

async fn run_headless() -> Result<HeadlessRuntimeInfo, HeadlessRuntimeError> {
    let config_path = cli_config_path()?;
    let data_dir = cli_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let config = AgentConfig::load(&config_path)?;
    if let AgentPortStatus::PrintBridge(agent) = check_agent_port(&config) {
        return Err(HeadlessRuntimeError::AlreadyRunning(agent));
    }

    let (listen_addr, listener) = server::bind_listener(&config).await?;
    let state = build_headless_state(config, config_path.clone())?;

    let info = HeadlessRuntimeInfo {
        config_path,
        data_dir,
        listen_addr,
    };

    let server_state = state.clone();
    let worker_state = state.clone();
    let remote_worker_state = state.clone();

    let server_task =
        tokio::spawn(async move { server::serve_listener(server_state, listener).await });
    tokio::spawn(async move {
        queue::run_worker(worker_state).await;
    });
    tokio::spawn(async move {
        remote_worker::run_worker(remote_worker_state).await;
    });

    println!("{}", format_headless_started(&info).trim_end());

    tokio::select! {
        server_result = server_task => {
            server_result??;
        }
        _ = shutdown_signal() => {}
    }

    Ok(info)
}

fn build_headless_state(
    config: AgentConfig,
    config_path: PathBuf,
) -> Result<AppState, HeadlessRuntimeError> {
    let remote_store = remote_store::RemoteStore::open(&cli_remote_store_path()?)?;
    let task_history = task_history::TaskHistoryStore::open(&cli_task_history_path()?)?;
    Ok(
        AppState::with_config_path_and_printing(config, config_path, printing::default_backend())
            .with_remote_store(remote_store)
            .with_task_history_store(task_history),
    )
}

/// 格式化 headless serve 启动信息。
pub fn format_headless_started(info: &HeadlessRuntimeInfo) -> String {
    format!(
        "PrintBridge serve started\nconfig: {}\ndata: {}\nlisten: {}\n",
        info.config_path.display(),
        info.data_dir.display(),
        info.listen_addr
    )
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_headless_started_includes_paths_and_listen_addr() {
        let info = HeadlessRuntimeInfo {
            config_path: PathBuf::from("/tmp/printbridge/config.json"),
            data_dir: PathBuf::from("/tmp/printbridge"),
            listen_addr: "127.0.0.1:17890".parse().unwrap(),
        };

        let output = format_headless_started(&info);

        assert!(output.contains("PrintBridge serve started"));
        assert!(output.contains("config: /tmp/printbridge/config.json"));
        assert!(output.contains("data: /tmp/printbridge"));
        assert!(output.contains("listen: 127.0.0.1:17890"));
    }

    #[test]
    fn already_running_error_is_readable() {
        let agent = crate::agent_guard::RunningAgent {
            addr: "127.0.0.1:17890".parse().unwrap(),
        };
        let error = HeadlessRuntimeError::AlreadyRunning(agent);

        assert_eq!(
            error.to_string(),
            "PrintBridge Agent is already running at 127.0.0.1:17890"
        );
    }
}
