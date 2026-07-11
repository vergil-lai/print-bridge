use std::io::{self, IsTerminal, Write};

use crate::{CommandError, CommandErrorKind};

/// CLI 密码输入与确认交互边界。
pub trait CliInteraction: Send + Sync {
    /// 读取不回显的密码。
    fn read_password(&self, prompt: &str) -> Result<String, CommandError>;

    /// 请求用户确认。
    fn confirm(&self, prompt: &str) -> Result<bool, CommandError>;

    /// 返回当前是否可进行交互。
    fn is_interactive(&self) -> bool;
}

/// 使用当前终端输入输出的交互实现。
pub struct TerminalInteraction;

impl CliInteraction for TerminalInteraction {
    fn read_password(&self, prompt: &str) -> Result<String, CommandError> {
        rpassword::prompt_password(prompt).map_err(interaction_error)
    }

    fn confirm(&self, prompt: &str) -> Result<bool, CommandError> {
        eprint!("{prompt} [y/N] ");
        io::stderr().flush().map_err(interaction_error)?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(interaction_error)?;
        Ok(matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    }

    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal() && io::stderr().is_terminal()
    }
}

fn interaction_error(error: impl ToString) -> CommandError {
    CommandError::new(CommandErrorKind::Runtime, error.to_string())
}
