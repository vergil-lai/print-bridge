use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use print_bridge_cli::{
    parser::run_cli_from, CliInteraction, Command, CommandError, CommandErrorKind, CommandExecutor,
    CommandResult, CommandService, DoctorCheck, DoctorReport, DoctorStatus, ProductCommandAdapter,
};
use print_bridge_core::config::AgentConfig;

struct Recorder(Arc<Mutex<Vec<Command>>>);

#[derive(Default)]
struct ProductRecorder {
    actions: Arc<Mutex<Vec<String>>>,
}

struct DoctorExecutor;

#[async_trait]
impl CommandExecutor for DoctorExecutor {
    async fn execute(&self, command: Command) -> Result<CommandResult, CommandError> {
        assert!(matches!(command, Command::Doctor { .. }));
        Ok(CommandResult::Doctor(DoctorReport::new(vec![
            DoctorCheck {
                code: "config.valid".into(),
                status: DoctorStatus::Fail,
                message: "invalid".into(),
                suggestion: Some("fix it".into()),
            },
        ])))
    }
}

#[async_trait]
impl ProductCommandAdapter for ProductRecorder {
    async fn autostart_status(&self) -> Result<bool, CommandError> {
        self.actions.lock().unwrap().push("autostart_status".into());
        Ok(true)
    }

    async fn set_autostart(&self, enabled: bool) -> Result<(), CommandError> {
        self.actions
            .lock()
            .unwrap()
            .push(format!("set_autostart:{enabled}"));
        Ok(())
    }

    async fn set_language(&self, language: &str) -> Result<(), CommandError> {
        self.actions
            .lock()
            .unwrap()
            .push(format!("set_language:{language}"));
        Ok(())
    }
}

fn product() -> Arc<dyn ProductCommandAdapter> {
    Arc::new(ProductRecorder::default())
}

struct TestInteraction;

impl CliInteraction for TestInteraction {
    fn read_password(&self, _prompt: &str) -> Result<String, CommandError> {
        Ok(String::new())
    }

    fn confirm(&self, _prompt: &str) -> Result<bool, CommandError> {
        Ok(true)
    }

    fn is_interactive(&self) -> bool {
        true
    }
}

fn interaction() -> Arc<dyn CliInteraction> {
    Arc::new(TestInteraction)
}

struct ConfigExecutor(Arc<Mutex<AgentConfig>>);

#[async_trait]
impl CommandExecutor for ConfigExecutor {
    async fn execute(&self, command: Command) -> Result<CommandResult, CommandError> {
        match command {
            Command::GetConfig => Ok(CommandResult::Config(Box::new(
                self.0.lock().unwrap().clone(),
            ))),
            Command::SaveConfig(config) => {
                *self.0.lock().unwrap() = config.clone();
                Ok(CommandResult::Config(Box::new(config)))
            }
            _ => panic!("unexpected command: {command:?}"),
        }
    }
}

fn config_service(config: Arc<Mutex<AgentConfig>>) -> Arc<CommandService> {
    let executor: Arc<dyn CommandExecutor> = Arc::new(ConfigExecutor(config));
    Arc::new(CommandService::new(None, executor))
}

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

    let output = run_cli_from(
        ["print-bridge", "printer"],
        service,
        product(),
        interaction(),
    )
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
    let error = run_cli_from(["print-bridge", "serve"], service, product(), interaction())
        .await
        .unwrap_err();
    assert!(error.message.contains("unrecognized subcommand 'serve'"));
}

#[tokio::test]
async fn desktop_commands_are_routed_through_product_adapter() {
    let executor: Arc<dyn CommandExecutor> = Arc::new(Recorder(Arc::new(Mutex::new(Vec::new()))));
    let service = Arc::new(CommandService::new(None, executor));
    let adapter = Arc::new(ProductRecorder::default());

    let output = run_cli_from(
        ["print-bridge", "autostart", "enable"],
        service.clone(),
        adapter.clone(),
        interaction(),
    )
    .await
    .unwrap();
    assert_eq!(output.stdout, "autostart enabled\n");

    run_cli_from(
        ["print-bridge", "app", "language", "zh-CN"],
        service,
        adapter.clone(),
        interaction(),
    )
    .await
    .unwrap();
    assert_eq!(
        *adapter.actions.lock().unwrap(),
        vec!["set_autostart:true", "set_language:zh-CN"]
    );
}

#[test]
fn command_errors_have_stable_exit_codes() {
    assert_eq!(
        CommandError::new(CommandErrorKind::InvalidInput, "").exit_code(),
        2
    );
    assert_eq!(
        CommandError::new(CommandErrorKind::NotRunning, "").exit_code(),
        3
    );
    assert_eq!(
        CommandError::new(CommandErrorKind::PermissionDenied, "").exit_code(),
        4
    );
    assert_eq!(
        CommandError::new(CommandErrorKind::Conflict, "").exit_code(),
        5
    );
    assert_eq!(
        CommandError::new(CommandErrorKind::Runtime, "").exit_code(),
        1
    );
    assert_eq!(
        CommandError::new(CommandErrorKind::Unsupported, "").exit_code(),
        1
    );
}

#[tokio::test]
async fn shared_configuration_commands_update_the_expected_fields() {
    let mut initial = AgentConfig::default();
    initial.service.port = 17521;
    let config = Arc::new(Mutex::new(initial));
    let service = config_service(config.clone());

    run_cli_from(
        ["print-bridge", "service", "set-port", "17521"],
        service.clone(),
        product(),
        interaction(),
    )
    .await
    .unwrap();
    run_cli_from(
        ["print-bridge", "ip", "add", "192.168.8.0/24"],
        service.clone(),
        product(),
        interaction(),
    )
    .await
    .unwrap();
    run_cli_from(
        ["print-bridge", "remote", "set-max-retries", "7"],
        service.clone(),
        product(),
        interaction(),
    )
    .await
    .unwrap();
    run_cli_from(
        ["print-bridge", "remote", "generate-device-id"],
        service,
        product(),
        interaction(),
    )
    .await
    .unwrap();

    let config = config.lock().unwrap();
    assert_eq!(config.service.port, 17521);
    assert_eq!(config.security.allowed_ips, ["127.0.0.1", "192.168.8.0/24"]);
    assert_eq!(config.remote.max_report_retries, 7);
    assert!(uuid::Uuid::parse_str(config.remote.device_id.as_deref().unwrap()).is_ok());
}

#[tokio::test]
async fn ip_delete_keeps_required_loopback_address() {
    let config = Arc::new(Mutex::new(AgentConfig::default()));
    let error = run_cli_from(
        ["print-bridge", "ip", "delete", "127.0.0.1"],
        config_service(config),
        product(),
        interaction(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind, CommandErrorKind::InvalidInput);
    assert!(error.message.contains("127.0.0.1"));
}

#[tokio::test]
async fn doctor_json_preserves_report_and_returns_one_on_fail() {
    let executor: Arc<dyn CommandExecutor> = Arc::new(DoctorExecutor);
    let service = Arc::new(CommandService::new(None, executor));
    let output = run_cli_from(
        ["print-bridge", "doctor", "--json"],
        service,
        product(),
        interaction(),
    )
    .await
    .unwrap();

    assert_eq!(output.exit_code, 1);
    assert!(output.stdout.contains("config.valid"));
    assert!(output.stdout.contains("\"fail\": 1"));
}
