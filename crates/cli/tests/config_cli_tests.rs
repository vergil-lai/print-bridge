use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use print_bridge_cli::{
    config_transfer::{ImportPreview, ImportPreviewItem},
    parser::run_cli_from,
    CliInteraction, Command, CommandError, CommandExecutor, CommandResult, CommandService,
    UnsupportedProductCommandAdapter,
};

struct Interaction {
    passwords: Mutex<VecDeque<String>>,
    interactive: bool,
}

impl Interaction {
    fn interactive(passwords: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            passwords: Mutex::new(passwords.iter().map(|value| (*value).into()).collect()),
            interactive: true,
        })
    }
}

impl CliInteraction for Interaction {
    fn read_password(&self, _prompt: &str) -> Result<String, CommandError> {
        Ok(self.passwords.lock().unwrap().pop_front().unwrap())
    }

    fn confirm(&self, _prompt: &str) -> Result<bool, CommandError> {
        Ok(true)
    }

    fn is_interactive(&self) -> bool {
        self.interactive
    }
}

struct Recorder(Arc<Mutex<Vec<Command>>>);

#[async_trait]
impl CommandExecutor for Recorder {
    async fn execute(&self, command: Command) -> Result<CommandResult, CommandError> {
        self.0.lock().unwrap().push(command.clone());
        match command {
            Command::GetConfig => Ok(CommandResult::Config(Box::default())),
            Command::ExportConfig { .. } | Command::ImportConfig { .. } => Ok(CommandResult::Empty),
            Command::PreviewConfigImport { .. } => {
                Ok(CommandResult::ImportPreview(ImportPreview {
                    file_hash: "preview-hash".into(),
                    items: vec![ImportPreviewItem {
                        key: "service.port".into(),
                        label: "Service port".into(),
                        current: "17890".into(),
                        next: "17521".into(),
                    }],
                }))
            }
            _ => panic!("unexpected command: {command:?}"),
        }
    }
}

fn service(commands: Arc<Mutex<Vec<Command>>>) -> Arc<CommandService> {
    let executor: Arc<dyn CommandExecutor> = Arc::new(Recorder(commands));
    Arc::new(CommandService::new(None, executor))
}

fn product() -> Arc<UnsupportedProductCommandAdapter> {
    Arc::new(UnsupportedProductCommandAdapter::new("unsupported"))
}

#[tokio::test]
async fn export_only_maps_to_selected_transfer_fields() {
    let commands = Arc::new(Mutex::new(Vec::new()));
    run_cli_from(
        [
            "print-bridge",
            "config",
            "export",
            "config.json",
            "--only",
            "service-port",
            "--only",
            "allowed-ips",
        ],
        service(commands.clone()),
        product(),
        Interaction::interactive(&["secret", "secret"]),
    )
    .await
    .unwrap();

    let commands = commands.lock().unwrap();
    let Command::ExportConfig { options, .. } = &commands[1] else {
        panic!("expected export command");
    };
    assert!(options.service_port);
    assert!(options.allowed_ips);
    assert!(!options.allowed_origins);
    assert!(!options.remote_bearer_token);
}

#[tokio::test]
async fn import_uses_the_hash_returned_by_preview() {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let output = run_cli_from(
        ["print-bridge", "config", "import", "config.json"],
        service(commands.clone()),
        product(),
        Interaction::interactive(&["secret"]),
    )
    .await
    .unwrap();

    assert!(!output.stdout.contains("secret"));
    assert!(matches!(
        commands.lock().unwrap().last(),
        Some(Command::ImportConfig { expected_file_hash, .. }) if expected_file_hash == "preview-hash"
    ));
}
