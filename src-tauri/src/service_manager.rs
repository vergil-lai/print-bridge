use crate::config::{cli_config_path, cli_data_dir, CONFIG_FILE_NAME, DATA_DIR_OVERRIDE_ENV};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};
use thiserror::Error;

const SYSTEMD_SERVICE_NAME: &str = "print-bridge.service";
const LAUNCH_AGENT_LABEL: &str = "com.printbridge.agent";

/// 需要执行的系统服务管理命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceCommand {
    /// 命令程序名。
    pub program: String,
    /// 命令参数。
    pub args: Vec<String>,
    /// 是否忽略非零退出码。
    pub allow_failure: bool,
}

/// 安装或卸载服务时的文件和命令计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePlan {
    /// 要写入或删除的托管服务文件路径。
    pub file_path: PathBuf,
    /// 安装时写入的文件内容。
    pub file_content: Option<String>,
    /// 卸载时删除的文件路径。
    pub remove_file_path: Option<PathBuf>,
    /// 写入或删除文件后执行的系统命令。
    pub commands: Vec<ServiceCommand>,
}

/// 服务安装或卸载完成后的用户可读输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceActionOutput {
    /// 展示给 CLI 用户的结果消息。
    pub message: String,
}

/// 安装或卸载系统托管服务时可能出现的错误。
#[derive(Debug, Error)]
pub enum ServiceManagerError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("serve install/uninstall is not supported on this platform")]
    UnsupportedPlatform,
    #[error("command failed: {command}\n{stderr}")]
    CommandFailed { command: String, stderr: String },
}

/// 安装或卸载 headless serve 托管服务的抽象，便于 CLI 测试注入 fake。
pub trait ServeServiceManager {
    /// 安装当前平台支持的 headless serve 托管服务。
    fn install(&self) -> Result<ServiceActionOutput, ServiceManagerError>;

    /// 卸载当前平台支持的 headless serve 托管服务。
    fn uninstall(&self) -> Result<ServiceActionOutput, ServiceManagerError>;
}

/// 使用当前平台的 systemd 或 launchd 管理 headless serve。
#[derive(Debug, Default)]
pub struct PlatformServeServiceManager;

impl ServeServiceManager for PlatformServeServiceManager {
    fn install(&self) -> Result<ServiceActionOutput, ServiceManagerError> {
        #[cfg(target_os = "linux")]
        {
            return install_linux_service();
        }

        #[cfg(target_os = "macos")]
        {
            install_macos_launch_agent()
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(ServiceManagerError::UnsupportedPlatform)
        }
    }

    fn uninstall(&self) -> Result<ServiceActionOutput, ServiceManagerError> {
        #[cfg(target_os = "linux")]
        {
            return uninstall_linux_service();
        }

        #[cfg(target_os = "macos")]
        {
            uninstall_macos_launch_agent()
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(ServiceManagerError::UnsupportedPlatform)
        }
    }
}

/// 生成 Linux systemd user service 文件内容。
pub fn render_systemd_service(executable: &Path, data_dir: &Path, config_path: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=PrintBridge Agent\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} serve\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         Environment=PRINT_BRIDGE_DATA_DIR={}\n\
         Environment=PRINT_BRIDGE_CONFIG_PATH={}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        executable.display(),
        data_dir.display(),
        config_path.display()
    )
}

/// 生成 macOS LaunchAgent plist 内容。
pub fn render_launch_agent(
    executable: &Path,
    data_dir: &Path,
    config_path: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{}</string>

  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>serve</string>
  </array>

  <key>EnvironmentVariables</key>
  <dict>
    <key>PRINT_BRIDGE_DATA_DIR</key>
    <string>{}</string>
    <key>PRINT_BRIDGE_CONFIG_PATH</key>
    <string>{}</string>
  </dict>

  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>

  <key>StandardOutPath</key>
  <string>{}</string>
  <key>StandardErrorPath</key>
  <string>{}</string>
</dict>
</plist>
"#,
        xml_escape(LAUNCH_AGENT_LABEL),
        xml_escape(&executable.to_string_lossy()),
        xml_escape(&data_dir.to_string_lossy()),
        xml_escape(&config_path.to_string_lossy()),
        xml_escape(&stdout_path.to_string_lossy()),
        xml_escape(&stderr_path.to_string_lossy())
    )
}

/// 生成 Linux systemd user service 安装计划。
pub fn linux_install_plan(
    service_path: &Path,
    executable: &Path,
    data_dir: &Path,
    config_path: &Path,
) -> ServicePlan {
    ServicePlan {
        file_path: service_path.to_path_buf(),
        file_content: Some(render_systemd_service(executable, data_dir, config_path)),
        remove_file_path: None,
        commands: vec![
            command("systemctl", ["--user", "daemon-reload"]),
            command(
                "systemctl",
                ["--user", "enable", "--now", SYSTEMD_SERVICE_NAME],
            ),
        ],
    }
}

/// 生成 Linux systemd user service 卸载计划。
pub fn linux_uninstall_plan(service_path: &Path) -> ServicePlan {
    ServicePlan {
        file_path: service_path.to_path_buf(),
        file_content: None,
        remove_file_path: Some(service_path.to_path_buf()),
        commands: vec![
            command(
                "systemctl",
                ["--user", "disable", "--now", SYSTEMD_SERVICE_NAME],
            ),
            command("systemctl", ["--user", "daemon-reload"]),
        ],
    }
}

/// 生成 macOS LaunchAgent 安装计划。
pub fn macos_install_plan(
    uid: u32,
    plist_path: &Path,
    executable: &Path,
    data_dir: &Path,
    config_path: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
) -> ServicePlan {
    let domain = format!("gui/{uid}");
    ServicePlan {
        file_path: plist_path.to_path_buf(),
        file_content: Some(render_launch_agent(
            executable,
            data_dir,
            config_path,
            stdout_path,
            stderr_path,
        )),
        remove_file_path: None,
        commands: vec![
            ServiceCommand {
                program: "launchctl".to_string(),
                args: vec![
                    "bootout".to_string(),
                    domain.clone(),
                    plist_path.display().to_string(),
                ],
                allow_failure: true,
            },
            ServiceCommand {
                program: "launchctl".to_string(),
                args: vec![
                    "bootstrap".to_string(),
                    domain.clone(),
                    plist_path.display().to_string(),
                ],
                allow_failure: false,
            },
            ServiceCommand {
                program: "launchctl".to_string(),
                args: vec![
                    "kickstart".to_string(),
                    "-k".to_string(),
                    format!("{domain}/{LAUNCH_AGENT_LABEL}"),
                ],
                allow_failure: false,
            },
        ],
    }
}

/// 生成 macOS LaunchAgent 卸载计划。
pub fn macos_uninstall_plan(uid: u32, plist_path: &Path) -> ServicePlan {
    let domain = format!("gui/{uid}");
    ServicePlan {
        file_path: plist_path.to_path_buf(),
        file_content: None,
        remove_file_path: Some(plist_path.to_path_buf()),
        commands: vec![ServiceCommand {
            program: "launchctl".to_string(),
            args: vec![
                "bootout".to_string(),
                domain,
                plist_path.display().to_string(),
            ],
            allow_failure: true,
        }],
    }
}

/// 构造默认不忽略失败的系统命令。
fn command<const N: usize>(program: &str, args: [&str; N]) -> ServiceCommand {
    ServiceCommand {
        program: program.to_string(),
        args: args.into_iter().map(str::to_string).collect(),
        allow_failure: false,
    }
}

/// 转义 LaunchAgent plist 中的 XML 文本节点。
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 执行一个服务安装或卸载计划。
fn apply_plan(plan: &ServicePlan) -> Result<(), ServiceManagerError> {
    if let Some(content) = plan.file_content.as_deref() {
        if let Some(parent) = plan.file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&plan.file_path, content)?;
    }

    for command in &plan.commands {
        run_service_command(command)?;
    }

    if let Some(path) = plan.remove_file_path.as_deref() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

/// 执行单条系统服务管理命令。
fn run_service_command(command: &ServiceCommand) -> Result<(), ServiceManagerError> {
    let output = Command::new(&command.program)
        .args(&command.args)
        .output()?;
    if output.status.success() || command.allow_failure {
        return Ok(());
    }

    Err(ServiceManagerError::CommandFailed {
        command: format!("{} {}", command.program, command.args.join(" ")),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

#[cfg(target_os = "linux")]
/// 安装 Linux systemd user service。
fn install_linux_service() -> Result<ServiceActionOutput, ServiceManagerError> {
    let plan = linux_install_plan(
        &linux_service_path()?,
        &env::current_exe()?,
        &service_data_dir()?,
        &service_config_path()?,
    );
    apply_plan(&plan)?;

    Ok(ServiceActionOutput {
        message: format!(
            "PrintBridge systemd user service installed: {}\n",
            plan.file_path.display()
        ),
    })
}

#[cfg(target_os = "linux")]
/// 卸载 Linux systemd user service。
fn uninstall_linux_service() -> Result<ServiceActionOutput, ServiceManagerError> {
    let plan = linux_uninstall_plan(&linux_service_path()?);
    apply_plan(&plan)?;

    Ok(ServiceActionOutput {
        message: format!(
            "PrintBridge systemd user service uninstalled: {}\n",
            plan.file_path.display()
        ),
    })
}

#[cfg(target_os = "macos")]
/// 安装 macOS LaunchAgent。
fn install_macos_launch_agent() -> Result<ServiceActionOutput, ServiceManagerError> {
    let paths = macos_paths()?;
    let plan = macos_install_plan(
        current_uid()?,
        &paths.plist_path,
        &env::current_exe()?,
        &service_data_dir()?,
        &service_config_path()?,
        &paths.stdout_path,
        &paths.stderr_path,
    );
    apply_plan(&plan)?;

    Ok(ServiceActionOutput {
        message: format!(
            "PrintBridge LaunchAgent installed: {}\n",
            plan.file_path.display()
        ),
    })
}

#[cfg(target_os = "macos")]
/// 卸载 macOS LaunchAgent。
fn uninstall_macos_launch_agent() -> Result<ServiceActionOutput, ServiceManagerError> {
    let paths = macos_paths()?;
    let plan = macos_uninstall_plan(current_uid()?, &paths.plist_path);
    apply_plan(&plan)?;

    Ok(ServiceActionOutput {
        message: format!(
            "PrintBridge LaunchAgent uninstalled: {}\n",
            plan.file_path.display()
        ),
    })
}

/// 返回托管服务使用的数据目录并确保目录存在。
fn service_data_dir() -> Result<PathBuf, io::Error> {
    let data_dir = cli_data_dir()?;
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir)
}

/// 返回托管服务写入服务文件的配置路径。
fn service_config_path() -> Result<PathBuf, io::Error> {
    if env::var_os(crate::config::CONFIG_PATH_OVERRIDE_ENV).is_some() {
        return cli_config_path();
    }

    if env::var_os(DATA_DIR_OVERRIDE_ENV).is_some() {
        return Ok(service_data_dir()?.join(CONFIG_FILE_NAME));
    }

    cli_config_path()
}

#[cfg(target_os = "linux")]
/// 返回 Linux systemd user service 文件路径。
fn linux_service_path() -> Result<PathBuf, io::Error> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".config"));
    Ok(base.join("systemd").join("user").join(SYSTEMD_SERVICE_NAME))
}

#[cfg(target_os = "macos")]
struct MacosPaths {
    plist_path: PathBuf,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

#[cfg(target_os = "macos")]
/// 返回 macOS LaunchAgent 文件和日志路径。
fn macos_paths() -> Result<MacosPaths, io::Error> {
    let home = home_dir()?;
    Ok(MacosPaths {
        plist_path: home
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCH_AGENT_LABEL}.plist")),
        stdout_path: home.join("Library").join("Logs").join("printbridge.log"),
        stderr_path: home
            .join("Library")
            .join("Logs")
            .join("printbridge.err.log"),
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
/// 返回当前用户 home 目录。
fn home_dir() -> Result<PathBuf, io::Error> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
}

#[cfg(target_os = "macos")]
/// 返回当前 macOS 登录用户的 uid。
fn current_uid() -> Result<u32, ServiceManagerError> {
    let output = Command::new("id").arg("-u").output()?;
    if !output.status.success() {
        return Err(ServiceManagerError::CommandFailed {
            command: "id -u".to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_service_runs_print_bridge_serve_with_cli_paths() {
        let unit = render_systemd_service(
            Path::new("/usr/local/bin/print-bridge"),
            Path::new("/var/lib/printbridge"),
            Path::new("/etc/printbridge/config.json"),
        );

        assert!(unit.contains("ExecStart=/usr/local/bin/print-bridge serve"));
        assert!(unit.contains("Environment=PRINT_BRIDGE_DATA_DIR=/var/lib/printbridge"));
        assert!(unit.contains("Environment=PRINT_BRIDGE_CONFIG_PATH=/etc/printbridge/config.json"));
        assert!(unit.contains("Restart=on-failure"));
    }

    #[test]
    fn launch_agent_escapes_xml_paths() {
        let plist = render_launch_agent(
            Path::new("/Applications/PrintBridge & Tools/print-bridge"),
            Path::new("/Users/alice/Library/Application Support/PrintBridge & Data"),
            Path::new("/Users/alice/Library/Application Support/PrintBridge/config.json"),
            Path::new("/Users/alice/Library/Logs/printbridge.log"),
            Path::new("/Users/alice/Library/Logs/printbridge.err.log"),
        );

        assert!(
            plist.contains("<string>/Applications/PrintBridge &amp; Tools/print-bridge</string>")
        );
        assert!(plist.contains(
            "<string>/Users/alice/Library/Application Support/PrintBridge &amp; Data</string>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn linux_install_plan_uses_systemd_user_commands() {
        let plan = linux_install_plan(
            Path::new("/home/alice/.config/systemd/user/print-bridge.service"),
            Path::new("/usr/local/bin/print-bridge"),
            Path::new("/home/alice/.config/com.vergil.printbridge"),
            Path::new("/home/alice/.config/com.vergil.printbridge/config.json"),
        );

        assert_eq!(
            plan.file_path,
            Path::new("/home/alice/.config/systemd/user/print-bridge.service")
        );
        assert_eq!(plan.commands.len(), 2);
        assert_eq!(plan.commands[0].program, "systemctl");
        assert_eq!(plan.commands[0].args, ["--user", "daemon-reload"]);
        assert_eq!(
            plan.commands[1].args,
            ["--user", "enable", "--now", "print-bridge.service"]
        );
    }

    #[test]
    fn macos_uninstall_plan_boots_out_and_removes_launch_agent() {
        let plan = macos_uninstall_plan(
            501,
            Path::new("/Users/alice/Library/LaunchAgents/com.printbridge.agent.plist"),
        );

        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].program, "launchctl");
        assert_eq!(
            plan.commands[0].args,
            [
                "bootout",
                "gui/501",
                "/Users/alice/Library/LaunchAgents/com.printbridge.agent.plist"
            ]
        );
        assert_eq!(
            plan.remove_file_path,
            Some(
                Path::new("/Users/alice/Library/LaunchAgents/com.printbridge.agent.plist")
                    .to_path_buf()
            )
        );
    }
}
