mod dependencies;
mod parser;
mod paths;
mod readiness;
mod signals;

use std::{net::SocketAddr, sync::Arc};

use clap::CommandFactory;
use parser::{ServerArgs, ServerCommand};
use print_bridge_cli::{
    client::LocalClientExecutor,
    parser::{run_cli_from, CliArgs},
    CommandExecutor, CommandService,
};
use print_bridge_runtime::{ipc, RuntimeBuilder, RuntimeCommandExecutor};

#[tokio::main]
async fn main() {
    let argv = std::env::args_os().collect::<Vec<_>>();
    if argv.len() == 1 {
        print!("{}", CliArgs::command().render_long_help());
        return;
    }
    let result = if argv.get(1).and_then(|arg| arg.to_str()) == Some("serve") {
        match ServerArgs::try_parse_product_from(argv) {
            Ok(args) if matches!(args.command, Some(ServerCommand::Serve)) => serve().await,
            Ok(_) => unreachable!(),
            Err(error) => error.exit(),
        }
    } else {
        run_shared_cli(argv).await
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    dependencies::preflight()?;
    let runtime = RuntimeBuilder::new(paths::system_paths()).build()?;
    let handle = runtime.start().await?;
    readiness::notify("READY=1")?;
    signals::shutdown_signal().await?;
    readiness::notify("STOPPING=1")?;
    handle.shutdown().await?;
    Ok(())
}

async fn run_shared_cli(argv: Vec<std::ffi::OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let paths = paths::system_paths();
    let runtime = RuntimeBuilder::new(paths.clone()).build()?;
    let offline: Arc<dyn CommandExecutor> = Arc::new(RuntimeCommandExecutor::new(
        runtime.state(),
        SocketAddr::from(([127, 0, 0, 1], 0)),
    ));
    let online: Arc<dyn CommandExecutor> = Arc::new(LocalClientExecutor::new(ipc::socket_path(
        &paths.runtime_dir,
    )));
    let output = run_cli_from(argv, Arc::new(CommandService::new(Some(online), offline))).await?;
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::parser::{ServerArgs, ServerCommand};
    use clap::Parser;

    #[test]
    fn no_args_shows_help_and_serve_is_explicit() {
        let no_args = ServerArgs::try_parse_product_from(["print-bridge"]).unwrap_err();
        assert_eq!(no_args.kind(), clap::error::ErrorKind::DisplayHelp);
        assert!(matches!(
            ServerArgs::try_parse_from(["print-bridge", "serve"])
                .unwrap()
                .command,
            Some(ServerCommand::Serve)
        ));
    }
}
