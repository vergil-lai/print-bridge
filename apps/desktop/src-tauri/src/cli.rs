use std::{net::SocketAddr, sync::Arc};

use print_bridge_cli::{
    client::LocalClientExecutor, parser::run_cli_from, CommandExecutor, CommandService,
};
use print_bridge_runtime::{ipc, RuntimeBuilder, RuntimeCommandExecutor, RuntimePaths};

/// 从当前进程参数运行两个产品共享的功能 CLI。
pub fn run_cli_from_env() -> i32 {
    match build_service() {
        Ok(service) => {
            let runtime = tokio::runtime::Runtime::new().expect("create CLI runtime");
            match runtime.block_on(run_cli_from(std::env::args_os(), service)) {
                Ok(output) => {
                    if !output.stdout.is_empty() {
                        print!("{}", output.stdout);
                    }
                    if !output.stderr.is_empty() {
                        eprint!("{}", output.stderr);
                    }
                    output.exit_code
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn build_service() -> Result<Arc<CommandService>, Box<dyn std::error::Error>> {
    let config_path = print_bridge_core::config::cli_config_path()?;
    let data_dir = print_bridge_core::config::cli_data_dir()?;
    let runtime_dir = data_dir.join("run");
    let runtime = RuntimeBuilder::new(RuntimePaths::new(
        config_path,
        data_dir,
        runtime_dir.clone(),
    ))
    .build()?;
    let offline: Arc<dyn CommandExecutor> = Arc::new(RuntimeCommandExecutor::new(
        runtime.state(),
        SocketAddr::from(([127, 0, 0, 1], 0)),
    ));
    let online: Arc<dyn CommandExecutor> =
        Arc::new(LocalClientExecutor::new(ipc::socket_path(&runtime_dir)));
    Ok(Arc::new(CommandService::new(Some(online), offline)))
}
