use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use print_bridge_cli::{
    parser::run_cli_from, Command, CommandError, CommandExecutor, CommandResult, CommandService,
};

struct Recorder(Arc<Mutex<Vec<Command>>>);

#[async_trait]
impl CommandExecutor for Recorder {
    async fn execute(&self, command: Command) -> Result<CommandResult, CommandError> {
        self.0.lock().unwrap().push(command);
        Ok(CommandResult::Printers(Vec::new()))
    }
}

#[tokio::test]
async fn printer_cli_uses_shared_list_printers_command() {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let executor: Arc<dyn CommandExecutor> = Arc::new(Recorder(commands.clone()));
    let service = Arc::new(CommandService::new(None, executor));

    let output = run_cli_from(["print-bridge", "printer"], service)
        .await
        .unwrap();

    assert_eq!(output.exit_code, 0);
    assert_eq!(*commands.lock().unwrap(), vec![Command::ListPrinters]);
}

#[tokio::test]
async fn shared_parser_rejects_serve() {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let executor: Arc<dyn CommandExecutor> = Arc::new(Recorder(commands));
    let service = Arc::new(CommandService::new(None, executor));
    let error = run_cli_from(["print-bridge", "serve"], service)
        .await
        .unwrap_err();
    assert!(error.message.contains("unrecognized subcommand 'serve'"));
}
