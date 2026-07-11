use std::sync::Arc;

use clap::{Args, CommandFactory, Parser, Subcommand};
use print_bridge_core::{
    config::AgentConfig,
    protocol::{validate_origin, EffectivePaper},
};

use crate::{Command, CommandError, CommandErrorKind, CommandResult, CommandService};

#[derive(Debug, Parser)]
#[command(name = "print-bridge", about = "PrintBridge CLI")]
pub struct CliArgs {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Status,
    Config,
    Printer(PrinterArgs),
    Paper(PaperArgs),
    Origin(OriginArgs),
    Remote(RemoteArgs),
    Task(TaskArgs),
    Logs,
    TestRemote,
    TestPrint,
}

#[derive(Debug, Args)]
struct PrinterArgs {
    #[command(subcommand)]
    command: Option<PrinterCommand>,
    printer_name: Option<String>,
}

#[derive(Debug, Subcommand)]
enum PrinterCommand {
    SetDefault { printer_name: String },
}

#[derive(Debug, Args)]
struct PaperArgs {
    #[command(subcommand)]
    command: Option<PaperCommand>,
}
#[derive(Debug, Subcommand)]
enum PaperCommand {
    Set { width: f64, height: f64 },
}

#[derive(Debug, Args)]
struct OriginArgs {
    #[command(subcommand)]
    command: Option<OriginCommand>,
}
#[derive(Debug, Subcommand)]
enum OriginCommand {
    Add { origin: String },
    Delete { origin: String },
}

#[derive(Debug, Args)]
struct RemoteArgs {
    #[command(subcommand)]
    command: Option<RemoteCommand>,
}
#[derive(Debug, Subcommand)]
enum RemoteCommand {
    Enable,
    Disable,
    SetUrl { url: String },
    SetToken { token: String },
    SetDeviceId { device_id: String },
    SetDeviceName { device_name: String },
    SetInterval { seconds: u64 },
}

#[derive(Debug, Args)]
struct TaskArgs {
    #[command(subcommand)]
    command: Option<TaskCommand>,
    job_id: Option<String>,
}
#[derive(Debug, Subcommand)]
enum TaskCommand {
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// 解析并通过共享 CommandService 执行功能 CLI。
pub async fn run_cli_from<I, T>(
    args: I,
    service: Arc<CommandService>,
) -> Result<CliOutput, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = match CliArgs::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            return Ok(CliOutput {
                stdout: error.to_string(),
                stderr: String::new(),
                exit_code: 0,
            });
        }
        Err(error) => {
            return Err(CommandError::new(
                CommandErrorKind::InvalidInput,
                error.to_string(),
            ));
        }
    };
    let stdout = match cli.command {
        None => CliArgs::command().render_long_help().to_string(),
        Some(command) => execute(command, &service).await?,
    };
    Ok(CliOutput {
        stdout,
        stderr: String::new(),
        exit_code: 0,
    })
}

async fn execute(command: CliCommand, service: &CommandService) -> Result<String, CommandError> {
    match command {
        CliCommand::Status => result_json(service.execute(Command::Status).await?),
        CliCommand::Config => result_json(service.execute(Command::GetConfig).await?),
        CliCommand::Printer(args) => printer(args, service).await,
        CliCommand::Paper(args) => paper(args, service).await,
        CliCommand::Origin(args) => origin(args, service).await,
        CliCommand::Remote(args) => remote(args, service).await,
        CliCommand::Task(args) => task(args, service).await,
        CliCommand::Logs => result_json(service.execute(Command::GetLogs).await?),
        CliCommand::TestRemote => {
            let config = get_config(service).await?;
            service
                .execute(Command::TestRemoteConnection { config })
                .await?;
            Ok("remote connection succeeded\n".into())
        }
        CliCommand::TestPrint => {
            let config = get_config(service).await?;
            service.execute(Command::TestPrint { config }).await?;
            Ok("test page submitted\n".into())
        }
    }
}

async fn printer(args: PrinterArgs, service: &CommandService) -> Result<String, CommandError> {
    match (args.command, args.printer_name) {
        (Some(PrinterCommand::SetDefault { printer_name }), None) => {
            let mut config = get_config(service).await?;
            config.printing.default_printer = Some(printer_name);
            save_config(service, config).await?;
            Ok("default printer updated\n".into())
        }
        (None, name) => match service.execute(Command::ListPrinters).await? {
            CommandResult::Printers(printers) => match name {
                Some(name) => json(
                    &printers
                        .into_iter()
                        .find(|item| item.name == name)
                        .ok_or_else(|| {
                            CommandError::new(CommandErrorKind::InvalidInput, "printer not found")
                        })?,
                ),
                None => json(&printers),
            },
            _ => unexpected(),
        },
        _ => Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "invalid printer command",
        )),
    }
}

async fn paper(args: PaperArgs, service: &CommandService) -> Result<String, CommandError> {
    match args.command {
        Some(PaperCommand::Set { width, height }) => {
            if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    "paper dimensions must be positive finite numbers",
                ));
            }
            let mut config = get_config(service).await?;
            config.printing.default_paper = Some(EffectivePaper {
                width_mm: width,
                height_mm: height,
            });
            save_config(service, config).await?;
            Ok("default paper updated\n".into())
        }
        None => {
            let config = get_config(service).await?;
            let printer_name = config.printing.default_printer.ok_or_else(|| {
                CommandError::new(
                    CommandErrorKind::InvalidInput,
                    "default printer is not configured",
                )
            })?;
            result_json(
                service
                    .execute(Command::ListPapers { printer_name })
                    .await?,
            )
        }
    }
}

async fn origin(args: OriginArgs, service: &CommandService) -> Result<String, CommandError> {
    let mut config = get_config(service).await?;
    match args.command {
        None => return json(&config.security.allowed_origins),
        Some(OriginCommand::Add { origin }) => {
            validate_origin(&origin).map_err(|error| {
                CommandError::new(CommandErrorKind::InvalidInput, error.to_string())
            })?;
            if !config.security.allowed_origins.contains(&origin) {
                config.security.allowed_origins.push(origin);
            }
        }
        Some(OriginCommand::Delete { origin }) => config
            .security
            .allowed_origins
            .retain(|item| item != &origin),
    }
    save_config(service, config).await?;
    Ok("allowed origins updated\n".into())
}

async fn remote(args: RemoteArgs, service: &CommandService) -> Result<String, CommandError> {
    let mut config = get_config(service).await?;
    match args.command {
        None => return json(&config.remote),
        Some(RemoteCommand::Enable) => config.remote.enabled = true,
        Some(RemoteCommand::Disable) => config.remote.enabled = false,
        Some(RemoteCommand::SetUrl { url }) => {
            let parsed = url::Url::parse(&url).map_err(|error| {
                CommandError::new(CommandErrorKind::InvalidInput, error.to_string())
            })?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    "remote URL must use http or https",
                ));
            }
            config.remote.endpoint_url = Some(url);
        }
        Some(RemoteCommand::SetToken { token }) => config.remote.bearer_token = Some(token),
        Some(RemoteCommand::SetDeviceId { device_id }) => config.remote.device_id = Some(device_id),
        Some(RemoteCommand::SetDeviceName { device_name }) => {
            config.remote.device_name = Some(device_name)
        }
        Some(RemoteCommand::SetInterval { seconds }) => {
            config.remote.poll_interval_seconds = seconds
        }
    }
    save_config(service, config).await?;
    Ok("remote configuration updated\n".into())
}

async fn task(args: TaskArgs, service: &CommandService) -> Result<String, CommandError> {
    match args.command {
        Some(TaskCommand::Clear) => {
            service.execute(Command::ClearTaskHistory).await?;
            Ok("task history cleared\n".into())
        }
        None => match args.job_id {
            Some(job_id) => result_json(
                service
                    .execute(Command::GetTaskHistoryEvents { job_id })
                    .await?,
            ),
            None => result_json(service.execute(Command::GetTaskHistory).await?),
        },
    }
}

async fn get_config(service: &CommandService) -> Result<AgentConfig, CommandError> {
    match service.execute(Command::GetConfig).await? {
        CommandResult::Config(config) => Ok(*config),
        _ => unexpected(),
    }
}

async fn save_config(service: &CommandService, config: AgentConfig) -> Result<(), CommandError> {
    match service.execute(Command::SaveConfig(config)).await? {
        CommandResult::Config(_) => Ok(()),
        _ => unexpected(),
    }
}

fn result_json(result: CommandResult) -> Result<String, CommandError> {
    json(&result)
}
fn json<T: serde::Serialize>(value: &T) -> Result<String, CommandError> {
    serde_json::to_string_pretty(value)
        .map(|value| value + "\n")
        .map_err(|error| CommandError::new(CommandErrorKind::Runtime, error.to_string()))
}
fn unexpected<T>() -> Result<T, CommandError> {
    Err(CommandError::new(
        CommandErrorKind::Runtime,
        "command returned an unexpected result",
    ))
}
