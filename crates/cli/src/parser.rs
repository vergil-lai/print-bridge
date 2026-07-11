use std::{path::PathBuf, sync::Arc};

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use print_bridge_core::{
    config::{AgentConfig, MAX_SERVICE_PORT, MIN_REMOTE_MAX_REPORT_RETRIES, MIN_SERVICE_PORT},
    ip_whitelist::{normalize_allowed_ips, validate_allowed_ip_entry, REQUIRED_LOOPBACK_IP},
    protocol::{validate_origin, EffectivePaper},
};

use crate::{
    config_transfer::ExportConfigOptions, CliInteraction, Command, CommandError, CommandErrorKind,
    CommandResult, CommandService, ProductCommandAdapter,
};

#[derive(Debug, Parser)]
#[command(name = "print-bridge", about = "PrintBridge CLI")]
pub struct CliArgs {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Status,
    Config(ConfigArgs),
    Printer(PrinterArgs),
    Paper(PaperArgs),
    Origin(OriginArgs),
    Remote(RemoteArgs),
    Task(TaskArgs),
    Logs,
    TestRemote,
    TestPrint,
    Autostart(AutostartArgs),
    App(AppArgs),
    Service(ServiceArgs),
    Ip(IpArgs),
    Doctor(DoctorArgs),
}

#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: Option<ConfigCommand>,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Export {
        path: PathBuf,
        #[arg(long, value_enum)]
        only: Vec<ExportField>,
        #[arg(long)]
        password_file: Option<PathBuf>,
        #[arg(long)]
        allow_empty_password: bool,
    },
    Import {
        path: PathBuf,
        #[arg(long)]
        password_file: Option<PathBuf>,
        #[arg(long)]
        preview: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        allow_empty_password: bool,
    },
    Validate {
        path: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ExportField {
    ServicePort,
    AllowedOrigins,
    AllowedIps,
    RemoteEnabled,
    RemoteUrl,
    RemoteToken,
    RemoteInterval,
    RemoteMaxRetries,
}

fn export_options(fields: &[ExportField]) -> ExportConfigOptions {
    if fields.is_empty() {
        return ExportConfigOptions::all();
    }
    ExportConfigOptions {
        service_port: fields
            .iter()
            .any(|field| matches!(field, ExportField::ServicePort)),
        allowed_origins: fields
            .iter()
            .any(|field| matches!(field, ExportField::AllowedOrigins)),
        allowed_ips: fields
            .iter()
            .any(|field| matches!(field, ExportField::AllowedIps)),
        remote_enabled: fields
            .iter()
            .any(|field| matches!(field, ExportField::RemoteEnabled)),
        remote_endpoint_url: fields
            .iter()
            .any(|field| matches!(field, ExportField::RemoteUrl)),
        remote_bearer_token: fields
            .iter()
            .any(|field| matches!(field, ExportField::RemoteToken)),
        remote_poll_interval_seconds: fields
            .iter()
            .any(|field| matches!(field, ExportField::RemoteInterval)),
        remote_max_report_retries: fields
            .iter()
            .any(|field| matches!(field, ExportField::RemoteMaxRetries)),
    }
}

#[derive(Debug, Args)]
struct ServiceArgs {
    #[command(subcommand)]
    command: Option<ServiceCommand>,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    SetPort { port: u16 },
}

#[derive(Debug, Args)]
struct IpArgs {
    #[command(subcommand)]
    command: Option<IpCommand>,
}

#[derive(Debug, Subcommand)]
enum IpCommand {
    Add { entry: String },
    Delete { entry: String },
}

#[derive(Debug, Args)]
struct AutostartArgs {
    #[command(subcommand)]
    command: Option<AutostartCommand>,
}

#[derive(Debug, Subcommand)]
enum AutostartCommand {
    Enable,
    Disable,
}

#[derive(Debug, Args)]
struct AppArgs {
    #[command(subcommand)]
    command: AppCommand,
}

#[derive(Debug, Subcommand)]
enum AppCommand {
    Language { language: String },
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
    SetMaxRetries { retries: u32 },
    GenerateDeviceId,
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
    product: Arc<dyn ProductCommandAdapter>,
    interaction: Arc<dyn CliInteraction>,
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
    let (stdout, exit_code) = match cli.command {
        None => (CliArgs::command().render_long_help().to_string(), 0),
        Some(CliCommand::Doctor(args)) => {
            let report = match service
                .execute(Command::Doctor {
                    product: product.product_kind(),
                })
                .await?
            {
                CommandResult::Doctor(report) => report,
                _ => return unexpected(),
            };
            let exit_code = i32::from(report.summary.fail > 0);
            let output = if args.json {
                json(&report)?
            } else {
                doctor_text(&report)
            };
            (output, exit_code)
        }
        Some(command) => (
            execute(command, &service, product.as_ref(), interaction.as_ref()).await?,
            0,
        ),
    };
    Ok(CliOutput {
        stdout,
        stderr: String::new(),
        exit_code,
    })
}

fn doctor_text(report: &crate::DoctorReport) -> String {
    use std::fmt::Write as _;

    let mut output = String::new();
    for check in &report.checks {
        let status = match check.status {
            crate::DoctorStatus::Pass => "PASS",
            crate::DoctorStatus::Warn => "WARN",
            crate::DoctorStatus::Fail => "FAIL",
        };
        let _ = writeln!(output, "[{status}] {}: {}", check.code, check.message);
        if let Some(suggestion) = &check.suggestion {
            let _ = writeln!(output, "  suggestion: {suggestion}");
        }
    }
    let _ = writeln!(
        output,
        "summary: {} pass, {} warn, {} fail",
        report.summary.pass, report.summary.warn, report.summary.fail
    );
    output
}

async fn execute(
    command: CliCommand,
    service: &CommandService,
    product: &dyn ProductCommandAdapter,
    interaction: &dyn CliInteraction,
) -> Result<String, CommandError> {
    match command {
        CliCommand::Status => result_json(service.execute(Command::Status).await?),
        CliCommand::Config(args) => config(args, service, interaction).await,
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
        CliCommand::Autostart(args) => match args.command {
            None => json(&serde_json::json!({
                "enabled": product.autostart_status().await?
            })),
            Some(AutostartCommand::Enable) => {
                product.set_autostart(true).await?;
                Ok("autostart enabled\n".into())
            }
            Some(AutostartCommand::Disable) => {
                product.set_autostart(false).await?;
                Ok("autostart disabled\n".into())
            }
        },
        CliCommand::App(args) => match args.command {
            AppCommand::Language { language } => {
                if !matches!(language.as_str(), "zh-CN" | "en") {
                    return Err(CommandError::new(
                        CommandErrorKind::InvalidInput,
                        "language must be zh-CN or en",
                    ));
                }
                product.set_language(&language).await?;
                Ok(
                    "application language updated; restart the GUI if it is currently open\n"
                        .into(),
                )
            }
        },
        CliCommand::Service(args) => service_config(args, service).await,
        CliCommand::Ip(args) => ip(args, service).await,
        CliCommand::Doctor(_) => unreachable!("doctor is handled before shared execution"),
    }
}

async fn config(
    args: ConfigArgs,
    service: &CommandService,
    interaction: &dyn CliInteraction,
) -> Result<String, CommandError> {
    match args.command {
        None => result_json(service.execute(Command::GetConfig).await?),
        Some(ConfigCommand::Validate { path }) => {
            service.execute(Command::ValidateConfig { path }).await?;
            Ok("configuration is valid\n".into())
        }
        Some(ConfigCommand::Export {
            path,
            only,
            password_file,
            allow_empty_password,
        }) => {
            let password = export_password(password_file.as_deref(), interaction)?;
            let config = get_config(service).await?;
            let options = export_options(&only);
            if password.is_empty()
                && options.remote_bearer_token
                && config.remote.bearer_token.is_some()
            {
                approve_empty_password(allow_empty_password, interaction)?;
            }
            service
                .execute(Command::ExportConfig {
                    path,
                    password,
                    options,
                })
                .await?;
            Ok("configuration exported\n".into())
        }
        Some(ConfigCommand::Import {
            path,
            password_file,
            preview,
            yes,
            allow_empty_password,
        }) => {
            if !interaction.is_interactive() && (password_file.is_none() || (!preview && !yes)) {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    "non-interactive import requires --password-file and --yes",
                ));
            }
            let password = import_password(password_file.as_deref(), interaction)?;
            if password.is_empty() && !interaction.is_interactive() && !allow_empty_password {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    "empty password requires --allow-empty-password",
                ));
            }
            let import_preview = match service
                .execute(Command::PreviewConfigImport {
                    path: path.clone(),
                    password: password.clone(),
                })
                .await?
            {
                CommandResult::ImportPreview(preview) => preview,
                _ => return unexpected(),
            };
            let rendered = json(&import_preview)?;
            if preview {
                return Ok(rendered);
            }
            if !yes && !interaction.confirm("Apply these configuration changes?")? {
                return Ok(rendered + "import cancelled\n");
            }
            service
                .execute(Command::ImportConfig {
                    path,
                    password,
                    expected_file_hash: import_preview.file_hash,
                })
                .await?;
            Ok(rendered + "configuration imported\n")
        }
    }
}

fn export_password(
    password_file: Option<&std::path::Path>,
    interaction: &dyn CliInteraction,
) -> Result<String, CommandError> {
    if let Some(path) = password_file {
        return read_password_file(path);
    }
    if !interaction.is_interactive() {
        return Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "non-interactive export requires --password-file",
        ));
    }
    let password = interaction.read_password("Password: ")?;
    let confirmation = interaction.read_password("Confirm password: ")?;
    if password != confirmation {
        return Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "passwords do not match",
        ));
    }
    Ok(password)
}

fn import_password(
    password_file: Option<&std::path::Path>,
    interaction: &dyn CliInteraction,
) -> Result<String, CommandError> {
    match password_file {
        Some(path) => read_password_file(path),
        None if interaction.is_interactive() => interaction.read_password("Password: "),
        None => Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "non-interactive import requires --password-file",
        )),
    }
}

fn read_password_file(path: &std::path::Path) -> Result<String, CommandError> {
    let mut bytes = std::fs::read(path).map_err(|error| {
        CommandError::new(
            CommandErrorKind::Runtime,
            format!("cannot read password file: {error}"),
        )
    })?;
    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    } else if bytes.ends_with(b"\n") {
        bytes.truncate(bytes.len() - 1);
    }
    String::from_utf8(bytes).map_err(|_| {
        CommandError::new(
            CommandErrorKind::InvalidInput,
            "password file must contain valid UTF-8",
        )
    })
}

fn approve_empty_password(
    explicitly_allowed: bool,
    interaction: &dyn CliInteraction,
) -> Result<(), CommandError> {
    if explicitly_allowed {
        return Ok(());
    }
    if interaction.is_interactive()
        && interaction
            .confirm("The export contains a bearer token and uses an empty password. Continue?")?
    {
        return Ok(());
    }
    Err(CommandError::new(
        CommandErrorKind::InvalidInput,
        "empty password with bearer token requires explicit confirmation or --allow-empty-password",
    ))
}

async fn service_config(
    args: ServiceArgs,
    service: &CommandService,
) -> Result<String, CommandError> {
    let mut config = get_config(service).await?;
    match args.command {
        None => json(&config.service),
        Some(ServiceCommand::SetPort { port }) => {
            if !(MIN_SERVICE_PORT..=MAX_SERVICE_PORT).contains(&port) {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    format!(
                        "service port must be between {MIN_SERVICE_PORT} and {MAX_SERVICE_PORT}"
                    ),
                ));
            }
            if port != config.service.port {
                std::net::TcpListener::bind(("0.0.0.0", port))
                    .map(drop)
                    .map_err(|_| {
                        CommandError::new(
                            CommandErrorKind::Conflict,
                            format!("service port {port} is already occupied"),
                        )
                    })?;
            }
            config.service.port = port;
            save_config(service, config).await?;
            Ok("service port updated; restart the agent to apply it\n".into())
        }
    }
}

async fn ip(args: IpArgs, service: &CommandService) -> Result<String, CommandError> {
    let mut config = get_config(service).await?;
    match args.command {
        None => return json(&config.security.allowed_ips),
        Some(IpCommand::Add { entry }) => {
            validate_allowed_ip_entry(&entry)
                .map_err(|error| CommandError::new(CommandErrorKind::InvalidInput, error))?;
            config.security.allowed_ips.push(entry);
            config.security.allowed_ips = normalize_allowed_ips(config.security.allowed_ips);
        }
        Some(IpCommand::Delete { entry }) => {
            if entry.trim() == REQUIRED_LOOPBACK_IP {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    format!("{REQUIRED_LOOPBACK_IP} is required and cannot be deleted"),
                ));
            }
            config
                .security
                .allowed_ips
                .retain(|item| item != entry.trim());
            config.security.allowed_ips = normalize_allowed_ips(config.security.allowed_ips);
        }
    }
    save_config(service, config).await?;
    Ok("allowed IPs updated\n".into())
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
        Some(RemoteCommand::SetMaxRetries { retries }) => {
            if retries < MIN_REMOTE_MAX_REPORT_RETRIES {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    format!("max retries must be at least {MIN_REMOTE_MAX_REPORT_RETRIES}"),
                ));
            }
            config.remote.max_report_retries = retries;
        }
        Some(RemoteCommand::GenerateDeviceId) => {
            config.remote.device_id = Some(uuid::Uuid::new_v4().to_string());
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
