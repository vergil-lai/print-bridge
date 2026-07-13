use std::{env, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use print_bridge_cli::{
    Command, CommandError, CommandErrorKind, CommandResult, CommandService, ProductCommandAdapter,
};
use print_bridge_core::config::{AgentConfig, UiLanguage};

/// Desktop 产品的系统集成 CLI 适配器。
pub struct DesktopProductCommandAdapter {
    service: Arc<CommandService>,
    autostart: AutoLaunch,
}

impl DesktopProductCommandAdapter {
    /// 使用当前桌面可执行文件创建产品适配器。
    pub fn new(service: Arc<CommandService>) -> Result<Self, CommandError> {
        let executable = desktop_executable_path().map_err(runtime_error)?;
        let mut builder = AutoLaunchBuilder::new();
        builder
            .set_app_name("PrintBridge")
            .set_app_path(&executable.to_string_lossy());
        #[cfg(target_os = "macos")]
        builder.set_use_launch_agent(false);
        let autostart = builder.build().map_err(runtime_error)?;
        Ok(Self { service, autostart })
    }

    async fn config(&self) -> Result<AgentConfig, CommandError> {
        match self.service.execute(Command::GetConfig).await? {
            CommandResult::Config(config) => Ok(*config),
            _ => Err(CommandError::new(
                CommandErrorKind::Runtime,
                "GetConfig returned an unexpected result",
            )),
        }
    }

    async fn save(&self, config: AgentConfig) -> Result<(), CommandError> {
        self.service.execute(Command::SaveConfig(config)).await?;
        Ok(())
    }
}

#[async_trait]
impl ProductCommandAdapter for DesktopProductCommandAdapter {
    async fn autostart_status(&self) -> Result<bool, CommandError> {
        self.autostart.is_enabled().map_err(runtime_error)
    }

    async fn set_autostart(&self, enabled: bool) -> Result<(), CommandError> {
        if enabled {
            self.autostart.enable()
        } else {
            self.autostart.disable()
        }
        .map_err(runtime_error)?;
        let mut config = self.config().await?;
        config.app.autostart = enabled;
        self.save(config).await
    }

    async fn set_language(&self, language: &str) -> Result<(), CommandError> {
        let language = match language {
            "zh-CN" => UiLanguage::ZhCn,
            "en" => UiLanguage::En,
            _ => {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    "language must be zh-CN or en",
                ))
            }
        };
        let mut config = self.config().await?;
        config.app.language = language;
        self.save(config).await
    }
}

fn desktop_executable_path() -> Result<PathBuf, std::io::Error> {
    #[cfg(target_os = "linux")]
    if let Some(path) = env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(path));
    }
    let executable = env::current_exe()?;
    #[cfg(target_os = "windows")]
    return Ok(windows_gui_executable_path(&executable));
    #[cfg(target_os = "macos")]
    {
        let executable = executable.canonicalize()?;
        let value = executable.to_string_lossy();
        if let Some((bundle, _)) = value.split_once(".app/") {
            return Ok(PathBuf::from(format!("{bundle}.app")));
        }
        Ok(executable)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(executable)
}

#[cfg(any(target_os = "windows", test))]
fn windows_gui_executable_path(executable: &std::path::Path) -> PathBuf {
    let value = executable.to_string_lossy();
    if let Some(index) = value.rfind(['\\', '/']) {
        return PathBuf::from(format!("{}PrintBridge.exe", &value[..=index]));
    }
    PathBuf::from("PrintBridge.exe")
}

fn runtime_error(error: impl ToString) -> CommandError {
    CommandError::new(CommandErrorKind::Runtime, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::windows_gui_executable_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn windows_cli_resolves_sibling_gui_for_autostart() {
        assert_eq!(
            windows_gui_executable_path(Path::new(
                r"C:\Users\me\AppData\Local\PrintBridge\print-bridge.exe"
            )),
            PathBuf::from(r"C:\Users\me\AppData\Local\PrintBridge\PrintBridge.exe")
        );
    }
}
