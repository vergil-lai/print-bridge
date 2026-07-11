use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use print_bridge_cli::{
    Command, CommandError, CommandErrorKind, CommandExecutor, CommandResult, CommandService,
};

struct RecordingExecutor {
    calls: Arc<Mutex<Vec<Command>>>,
    result: Result<CommandResult, CommandError>,
}

#[async_trait]
impl CommandExecutor for RecordingExecutor {
    async fn execute(&self, command: Command) -> Result<CommandResult, CommandError> {
        self.calls.lock().unwrap().push(command);
        self.result.clone()
    }
}

fn executor(
    result: Result<CommandResult, CommandError>,
) -> (Arc<dyn CommandExecutor>, Arc<Mutex<Vec<Command>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Arc::new(RecordingExecutor {
            calls: calls.clone(),
            result,
        }),
        calls,
    )
}

#[tokio::test]
async fn online_preferred_falls_back_only_when_agent_is_not_running() {
    let (online, online_calls) = executor(Err(CommandError::new(
        CommandErrorKind::NotRunning,
        "agent is not running",
    )));
    let (offline, offline_calls) = executor(Ok(CommandResult::Empty));
    let service = CommandService::new(Some(online), offline);

    let result = service.execute(Command::ClearTaskHistory).await;

    assert_eq!(result, Ok(CommandResult::Empty));
    assert_eq!(online_calls.lock().unwrap().len(), 1);
    assert_eq!(offline_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn online_preferred_does_not_hide_runtime_errors() {
    let expected = CommandError::new(CommandErrorKind::Runtime, "ipc failed");
    let (online, _) = executor(Err(expected.clone()));
    let (offline, offline_calls) = executor(Ok(CommandResult::Empty));
    let service = CommandService::new(Some(online), offline);

    let result = service.execute(Command::ClearTaskHistory).await;

    assert_eq!(result, Err(expected));
    assert!(offline_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn online_only_reports_not_running_without_an_online_executor() {
    let (offline, offline_calls) = executor(Ok(CommandResult::Empty));
    let service = CommandService::new(None, offline);

    let error = service.execute(Command::Status).await.unwrap_err();

    assert_eq!(error.kind, CommandErrorKind::NotRunning);
    assert!(offline_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn offline_allowed_uses_the_offline_executor() {
    let (online, online_calls) = executor(Ok(CommandResult::Empty));
    let (offline, offline_calls) = executor(Ok(CommandResult::Empty));
    let service = CommandService::new(Some(online), offline);

    let result = service
        .execute(Command::ValidateConfig { path: None })
        .await;

    assert_eq!(result, Ok(CommandResult::Empty));
    assert!(online_calls.lock().unwrap().is_empty());
    assert_eq!(offline_calls.lock().unwrap().len(), 1);
}
