use std::sync::Arc;

use async_trait::async_trait;

use crate::{Command, CommandError, CommandErrorKind, CommandPolicy, CommandResult};

/// 执行命令的在线或离线端口。
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn execute(&self, command: Command) -> Result<CommandResult, CommandError>;
}

/// 根据命令策略选择在线 Agent 或离线实现。
pub struct CommandService {
    online: Option<Arc<dyn CommandExecutor>>,
    offline: Arc<dyn CommandExecutor>,
}

impl CommandService {
    /// 创建共享命令服务。
    pub fn new(
        online: Option<Arc<dyn CommandExecutor>>,
        offline: Arc<dyn CommandExecutor>,
    ) -> Self {
        Self { online, offline }
    }

    /// 按命令策略执行，并仅对明确的未运行错误执行离线回退。
    pub async fn execute(&self, command: Command) -> Result<CommandResult, CommandError> {
        match command.policy() {
            CommandPolicy::OfflineAllowed => self.offline.execute(command).await,
            CommandPolicy::OnlineOnly => {
                let online = self.online.as_ref().ok_or_else(not_running_error)?;
                online.execute(command).await
            }
            CommandPolicy::OnlinePreferred => {
                let Some(online) = self.online.as_ref() else {
                    return self.offline.execute(command).await;
                };
                match online.execute(command.clone()).await {
                    Err(error) if error.kind == CommandErrorKind::NotRunning => {
                        self.offline.execute(command).await
                    }
                    result => result,
                }
            }
        }
    }
}

fn not_running_error() -> CommandError {
    CommandError::new(CommandErrorKind::NotRunning, "agent is not running")
}
