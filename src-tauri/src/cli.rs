use crate::{
    config::{cli_config_path, cli_task_history_path, AgentConfig},
    printing::{default_backend, PrintBackend, PrinterInfo},
    protocol::{validate_origin, EffectivePaper},
    task_history::{TaskHistoryEvent, TaskHistoryJob, TaskHistoryStore},
};
use clap::{error::ErrorKind, Args, CommandFactory, Parser, Subcommand};
use std::{fmt, io};
use url::Url;

/// CLI 命令执行统一返回结果。
pub type CliResult<T> = Result<T, CliError>;

/// CLI 命令执行后的标准输出、错误输出和退出码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl CliOutput {
    fn ok(stdout: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: 0,
        }
    }
}

/// CLI 参数解析、配置读取和打印操作返回的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    Help(String),
    Usage(String),
    Printing(String),
    Config(String),
    TaskHistory(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Help(message) => formatter.write_str(message),
            Self::Usage(message) => formatter.write_str(message),
            Self::Printing(message) => formatter.write_str(message),
            Self::Config(message) => formatter.write_str(message),
            Self::TaskHistory(message) => formatter.write_str(message),
        }
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::Printing(error.to_string())
    }
}

#[derive(Debug, Parser)]
#[command(name = "print-bridge", about = "PrintBridge CLI")]
struct PrintBridgeCli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Get printer list or printer information.
    Printer(PrinterArgs),
    /// Get or set default paper.
    Paper(PaperArgs),
    /// Get or update allowed origins.
    Origin(OriginArgs),
    /// Get or update remote task settings.
    Remote(RemoteArgs),
    /// Get or clear local task history.
    Task(TaskArgs),
}

#[derive(Debug, Args)]
struct PrinterArgs {
    #[command(subcommand)]
    command: Option<PrinterCommand>,
    printer_name: Option<String>,
}

#[derive(Debug, Subcommand)]
enum PrinterCommand {
    /// Set default printer.
    SetDefault { printer_name: String },
}

#[derive(Debug, Args)]
struct PaperArgs {
    #[command(subcommand)]
    command: Option<PaperCommand>,
}

#[derive(Debug, Subcommand)]
enum PaperCommand {
    /// Set default paper size in millimeters.
    Set { width: String, height: String },
}

#[derive(Debug, Args)]
struct OriginArgs {
    #[command(subcommand)]
    command: Option<OriginCommand>,
}

#[derive(Debug, Subcommand)]
enum OriginCommand {
    /// Add an allowed origin.
    Add { origin: String },
    /// Delete an allowed origin.
    Delete { origin: String },
}

#[derive(Debug, Args)]
struct RemoteArgs {
    #[command(subcommand)]
    command: Option<RemoteCommand>,
}

#[derive(Debug, Subcommand)]
enum RemoteCommand {
    /// Enable remote task polling.
    Enable,
    /// Disable remote task polling.
    Disable,
    /// Set remote task endpoint URL.
    SetUrl { url: String },
    /// Set remote task bearer token.
    SetToken { token: String },
    /// Set remote device ID.
    SetDeviceId { device_id: String },
    /// Set remote device name.
    SetDeviceName { device_name: String },
    /// Set remote polling interval in seconds.
    SetInterval { seconds: String },
}

#[derive(Debug, Args)]
struct TaskArgs {
    #[command(subcommand)]
    command: Option<TaskCommand>,
    job_id: Option<String>,
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    /// Clear local task history.
    Clear,
}

/// 使用传入参数执行 CLI，便于测试和桌面入口复用。
pub fn run_cli_from<I, S>(args: I) -> CliResult<CliOutput>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let backend = default_backend();
    run_cli_with_backend(args, backend.as_ref())
}

fn run_cli_with_backend<I, S>(
    args: I,
    backend: &(dyn PrintBackend + Send + Sync),
) -> CliResult<CliOutput>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let cli = match parse_cli(args) {
        Ok(cli) => cli,
        Err(CliError::Help(message)) => return Ok(CliOutput::ok(message)),
        Err(error) => return Err(error),
    };

    match cli.command {
        Some(CliCommand::Printer(args)) => handle_printer_command(args, backend),
        Some(CliCommand::Paper(args)) => handle_paper_command(args, backend),
        Some(CliCommand::Origin(args)) => handle_origin_command(args),
        Some(CliCommand::Remote(args)) => handle_remote_command(args),
        Some(CliCommand::Task(args)) => handle_task_command(args),
        None => Ok(CliOutput::ok(help_text())),
    }
}

/// 从当前进程参数执行 CLI 并返回进程退出码。
pub fn run_cli_from_env() -> i32 {
    match run_cli_from(std::env::args().skip(1)) {
        Ok(output) => {
            if !output.stdout.is_empty() {
                println!("{}", output.stdout.trim_end());
            }
            if !output.stderr.is_empty() {
                eprintln!("{}", output.stderr.trim_end());
            }
            output.exit_code
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn help_text() -> String {
    PrintBridgeCli::command().render_long_help().to_string()
}

fn parse_cli(args: Vec<String>) -> CliResult<PrintBridgeCli> {
    let argv = std::iter::once("print-bridge".to_string()).chain(args);
    match PrintBridgeCli::try_parse_from(argv) {
        Ok(cli) => Ok(cli),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            Err(CliError::Help(error.to_string()))
        }
        Err(error) => Err(CliError::Usage(error.to_string())),
    }
}

fn handle_printer_command(
    args: PrinterArgs,
    backend: &(dyn PrintBackend + Send + Sync),
) -> CliResult<CliOutput> {
    match (args.command, args.printer_name) {
        (Some(PrinterCommand::SetDefault { printer_name }), None) => {
            handle_printer(&["set-default".to_string(), printer_name], backend)
        }
        (None, Some(printer_name)) => handle_printer(&[printer_name], backend),
        (None, None) => handle_printer(&[], backend),
        _ => Err(CliError::Usage(
            "usage: print-bridge printer [printer-name]|set-default <printer-name>".to_string(),
        )),
    }
}

fn handle_paper_command(
    args: PaperArgs,
    backend: &(dyn PrintBackend + Send + Sync),
) -> CliResult<CliOutput> {
    match args.command {
        Some(PaperCommand::Set { width, height }) => {
            handle_paper(&["set".to_string(), width, height], backend)
        }
        None => handle_paper(&[], backend),
    }
}

fn handle_origin_command(args: OriginArgs) -> CliResult<CliOutput> {
    match args.command {
        Some(OriginCommand::Add { origin }) => handle_origin(&["add".to_string(), origin]),
        Some(OriginCommand::Delete { origin }) => handle_origin(&["delete".to_string(), origin]),
        None => handle_origin(&[]),
    }
}

fn handle_remote_command(args: RemoteArgs) -> CliResult<CliOutput> {
    match args.command {
        Some(RemoteCommand::Enable) => handle_remote(&["enable".to_string()]),
        Some(RemoteCommand::Disable) => handle_remote(&["disable".to_string()]),
        Some(RemoteCommand::SetUrl { url }) => handle_remote(&["set-url".to_string(), url]),
        Some(RemoteCommand::SetToken { token }) => handle_remote(&["set-token".to_string(), token]),
        Some(RemoteCommand::SetDeviceId { device_id }) => {
            handle_remote(&["set-device-id".to_string(), device_id])
        }
        Some(RemoteCommand::SetDeviceName { device_name }) => {
            handle_remote(&["set-device-name".to_string(), device_name])
        }
        Some(RemoteCommand::SetInterval { seconds }) => {
            handle_remote(&["set-interval".to_string(), seconds])
        }
        None => handle_remote(&[]),
    }
}

fn handle_task_command(args: TaskArgs) -> CliResult<CliOutput> {
    match (args.command, args.job_id) {
        (Some(TaskCommand::Clear), None) => handle_task(&["clear".to_string()]),
        (None, Some(job_id)) => handle_task(&[job_id]),
        (None, None) => handle_task(&[]),
        _ => Err(CliError::Usage(
            "usage: print-bridge task [job-id]|clear".to_string(),
        )),
    }
}

fn handle_printer(
    args: &[String],
    backend: &(dyn PrintBackend + Send + Sync),
) -> CliResult<CliOutput> {
    match args {
        [] => {
            let printers = backend
                .list_printers()
                .map_err(|error| CliError::Printing(error.to_string()))?;
            Ok(CliOutput::ok(format_printer_list(&printers)))
        }
        [action, printer_name] if action == "set-default" => {
            let printers = backend
                .list_printers()
                .map_err(|error| CliError::Printing(error.to_string()))?;
            if !printers.iter().any(|printer| printer.name == *printer_name) {
                return Err(CliError::Printing(format!(
                    "printer not found: {printer_name}"
                )));
            }

            let path = cli_config_path()?;
            let mut config = AgentConfig::load(&path)?;
            config.printing.default_printer = Some(printer_name.clone());
            config.save(&path)?;

            Ok(CliOutput::ok(format!(
                "default printer set to {printer_name}\n"
            )))
        }
        [printer_name] => {
            let printers = backend
                .list_printers()
                .map_err(|error| CliError::Printing(error.to_string()))?;
            let printer = printers
                .iter()
                .find(|printer| printer.name == *printer_name)
                .ok_or_else(|| CliError::Printing(format!("printer not found: {printer_name}")))?;
            Ok(CliOutput::ok(format_printer_detail(printer)))
        }
        _ => Err(CliError::Usage(
            "usage: print-bridge printer [printer-name]|set-default <printer-name>".to_string(),
        )),
    }
}

fn format_printer_list(printers: &[PrinterInfo]) -> String {
    if printers.is_empty() {
        return "No printers found.\n".to_string();
    }

    printers
        .iter()
        .map(|printer| {
            let marker = if printer.is_default { "*" } else { " " };
            format!("{marker} {}", printer.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn format_printer_detail(printer: &PrinterInfo) -> String {
    format!(
        "name: {}\ndefault: {}\ndpi: {}\nport: {}\nlocal: {}\nnetwork: {}\nvirtual: {}\n",
        printer.name,
        printer.is_default,
        optional_u32(printer.dpi),
        optional_str(printer.port.as_deref()),
        optional_bool(printer.is_local),
        optional_bool(printer.is_network),
        optional_bool(printer.is_virtual)
    )
}

fn optional_u32(value: Option<u32>) -> String {
    value
        .map(|item| item.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn optional_str(value: Option<&str>) -> String {
    value.unwrap_or("-").to_string()
}

fn optional_bool(value: Option<bool>) -> String {
    value
        .map(|item| item.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn handle_paper(
    args: &[String],
    backend: &(dyn PrintBackend + Send + Sync),
) -> CliResult<CliOutput> {
    match args {
        [] => {
            let path = cli_config_path()?;
            let config = AgentConfig::load(&path)?;
            Ok(CliOutput::ok(format_paper_config(&config, backend)?))
        }
        [action, width, height] if action == "set" => {
            let width_mm = parse_positive_f64(width)?;
            let height_mm = parse_positive_f64(height)?;
            let path = cli_config_path()?;
            let mut config = AgentConfig::load(&path)?;
            config.printing.default_paper = Some(EffectivePaper {
                width_mm,
                height_mm,
            });
            config.save(&path)?;

            Ok(CliOutput::ok(format!(
                "default paper set to {} x {} mm\n",
                trim_float(width_mm),
                trim_float(height_mm)
            )))
        }
        _ => Err(CliError::Usage(
            "usage: print-bridge paper [set <width-mm> <height-mm>]".to_string(),
        )),
    }
}

fn handle_origin(args: &[String]) -> CliResult<CliOutput> {
    let path = cli_config_path()?;
    let mut config = AgentConfig::load(&path)?;

    match args {
        [] => Ok(CliOutput::ok(format_origin_list(
            &config.security.allowed_origins,
        ))),
        [action, origin] if action == "add" => {
            validate_origin(origin).map_err(|error| CliError::Config(error.to_string()))?;
            if !config
                .security
                .allowed_origins
                .iter()
                .any(|item| item == origin)
            {
                config.security.allowed_origins.push(origin.clone());
                config.save(&path)?;
            }
            Ok(CliOutput::ok(format!("origin added: {origin}\n")))
        }
        [action, origin] if action == "delete" => {
            let before = config.security.allowed_origins.len();
            config
                .security
                .allowed_origins
                .retain(|item| item != origin);
            if config.security.allowed_origins.len() != before {
                config.save(&path)?;
                Ok(CliOutput::ok(format!("origin deleted: {origin}\n")))
            } else {
                Ok(CliOutput::ok(format!("origin not found: {origin}\n")))
            }
        }
        _ => Err(CliError::Usage(
            "usage: print-bridge origin [add|delete <origin>]".to_string(),
        )),
    }
}

fn format_origin_list(origins: &[String]) -> String {
    if origins.is_empty() {
        return "No allowed origins configured.\n".to_string();
    }

    origins.join("\n") + "\n"
}

fn handle_remote(args: &[String]) -> CliResult<CliOutput> {
    let path = cli_config_path()?;
    let mut config = AgentConfig::load(&path)?;

    match args {
        [] => Ok(CliOutput::ok(format_remote_config(&config))),
        [action] if action == "enable" => {
            config.remote.enabled = true;
            config.save(&path)?;
            Ok(CliOutput::ok("remote enabled\n"))
        }
        [action] if action == "disable" => {
            config.remote.enabled = false;
            config.save(&path)?;
            Ok(CliOutput::ok("remote disabled\n"))
        }
        [action, value] if action == "set-url" => {
            config.remote.endpoint_url = normalize_remote_url(value)?;
            config.save(&path)?;
            Ok(CliOutput::ok("remote url updated\n"))
        }
        [action, value] if action == "set-token" => {
            config.remote.bearer_token = empty_to_none(value);
            config.save(&path)?;
            Ok(CliOutput::ok("remote token updated\n"))
        }
        [action, value] if action == "set-device-id" => {
            config.remote.device_id = empty_to_none(value);
            config.save(&path)?;
            Ok(CliOutput::ok("remote device id updated\n"))
        }
        [action, value] if action == "set-device-name" => {
            config.remote.device_name = empty_to_none(value);
            config.save(&path)?;
            Ok(CliOutput::ok("remote device name updated\n"))
        }
        [action, value] if action == "set-interval" => {
            let seconds = value.parse::<u64>().map_err(|_| {
                CliError::Usage("remote interval must be a positive integer".to_string())
            })?;
            if seconds == 0 {
                return Err(CliError::Usage(
                    "remote interval must be a positive integer".to_string(),
                ));
            }
            config.remote.poll_interval_seconds = seconds;
            config.save(&path)?;
            Ok(CliOutput::ok("remote interval updated\n"))
        }
        _ => Err(CliError::Usage(
            "usage: print-bridge remote [enable|disable|set-url|set-token|set-device-id|set-device-name|set-interval]"
                .to_string(),
        )),
    }
}

fn handle_task(args: &[String]) -> CliResult<CliOutput> {
    let path = cli_task_history_path()?;
    if !path.exists() {
        return Ok(CliOutput::ok("No task history found.\n"));
    }

    let store =
        TaskHistoryStore::open(&path).map_err(|error| CliError::TaskHistory(error.to_string()))?;

    match args {
        [] => {
            let jobs = store
                .recent_jobs(50)
                .map_err(|error| CliError::TaskHistory(error.to_string()))?;
            Ok(CliOutput::ok(format_task_list(&jobs)))
        }
        [action] if action == "clear" => {
            store
                .clear()
                .map_err(|error| CliError::TaskHistory(error.to_string()))?;
            Ok(CliOutput::ok("task history cleared\n"))
        }
        [job_id] => {
            let events = store
                .events_for_job(job_id)
                .map_err(|error| CliError::TaskHistory(error.to_string()))?;
            Ok(CliOutput::ok(format_task_detail(job_id, &events)))
        }
        _ => Err(CliError::Usage(
            "usage: print-bridge task [job-id]|clear".to_string(),
        )),
    }
}

fn format_task_list(jobs: &[TaskHistoryJob]) -> String {
    if jobs.is_empty() {
        return "No task history found.\n".to_string();
    }

    jobs.iter()
        .map(|job| {
            format!(
                "{} {} {} {}",
                job.updated_at,
                job.job_id,
                job.current_status.as_str(),
                job.current_message.as_deref().unwrap_or("-")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn format_task_detail(job_id: &str, events: &[TaskHistoryEvent]) -> String {
    if events.is_empty() {
        return format!("No task events found for {job_id}.\n");
    }

    let mut lines = vec![format!("job_id: {job_id}")];
    for event in events {
        lines.push(format!(
            "{} {} {}",
            event.occurred_at,
            event.status.as_str(),
            event.message.as_deref().unwrap_or("-")
        ));
    }

    lines.join("\n") + "\n"
}

fn empty_to_none(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn normalize_remote_url(value: &str) -> CliResult<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }

    let url = Url::parse(value).map_err(|_| CliError::Config("invalid remote url".to_string()))?;
    match url.scheme() {
        "http" | "https" => Ok(Some(value.to_string())),
        _ => Err(CliError::Config(
            "remote url must use http or https".to_string(),
        )),
    }
}

fn format_remote_config(config: &AgentConfig) -> String {
    format!(
        "enabled: {}\nurl: {}\ntoken: {}\ndevice_id: {}\ndevice_name: {}\ninterval_seconds: {}\n",
        config.remote.enabled,
        optional_str(config.remote.endpoint_url.as_deref()),
        if config.remote.bearer_token.is_some() {
            "set"
        } else {
            "-"
        },
        optional_str(config.remote.device_id.as_deref()),
        optional_str(config.remote.device_name.as_deref()),
        config.remote.poll_interval_seconds
    )
}

fn format_paper_config(
    config: &AgentConfig,
    backend: &(dyn PrintBackend + Send + Sync),
) -> CliResult<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "default printer: {}",
        optional_str(config.printing.default_printer.as_deref())
    ));
    lines.push(format!(
        "default paper: {}",
        config
            .printing
            .default_paper
            .as_ref()
            .map(|paper| format!(
                "{} x {} mm",
                trim_float(paper.width_mm),
                trim_float(paper.height_mm)
            ))
            .unwrap_or_else(|| "-".to_string())
    ));

    if let Some(printer_name) = config.printing.default_printer.as_deref() {
        let papers = backend
            .list_papers(printer_name)
            .map_err(|error| CliError::Printing(error.to_string()))?;
        lines.push("available papers:".to_string());
        if papers.is_empty() {
            lines.push("  -".to_string());
        } else {
            for paper in papers {
                lines.push(format!(
                    "  {}: {} x {} mm",
                    paper.name,
                    trim_float(paper.width_mm),
                    trim_float(paper.height_mm)
                ));
            }
        }
    }

    Ok(lines.join("\n") + "\n")
}

fn parse_positive_f64(value: &str) -> CliResult<f64> {
    let parsed = value.parse::<f64>().map_err(|_| {
        CliError::Usage("paper width and height must be positive numbers".to_string())
    })?;
    if parsed.is_finite() && parsed > 0.0 {
        Ok(parsed)
    } else {
        Err(CliError::Usage(
            "paper width and height must be positive numbers".to_string(),
        ))
    }
}

fn trim_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CONFIG_PATH_OVERRIDE_ENV, DATA_DIR_OVERRIDE_ENV};
    use crate::printing::PaperInfo;
    use crate::task_history::{NewTaskHistoryEvent, TaskHistorySource, TaskHistoryStatus};
    use std::{
        env,
        ffi::OsString,
        fs,
        sync::{Mutex, OnceLock},
    };
    use uuid::Uuid;

    #[derive(Debug)]
    struct FakePrintBackend {
        printers: Vec<PrinterInfo>,
        papers: Vec<PaperInfo>,
    }

    impl PrintBackend for FakePrintBackend {
        fn list_printers(&self) -> crate::printing::PrintResult<Vec<PrinterInfo>> {
            Ok(self.printers.clone())
        }

        fn list_papers(&self, _printer_name: &str) -> crate::printing::PrintResult<Vec<PaperInfo>> {
            Ok(self.papers.clone())
        }

        fn print_pdf(
            &self,
            _path: &std::path::Path,
            _options: &crate::printing::PrintOptions,
        ) -> crate::printing::PrintResult<crate::printing::PrintSubmission> {
            Err(crate::printing::PrintError::UnsupportedPlatform)
        }

        fn print_raw(
            &self,
            _data: &[u8],
            _options: &crate::printing::RawPrintOptions,
        ) -> crate::printing::PrintResult<crate::printing::PrintSubmission> {
            Err(crate::printing::PrintError::UnsupportedPlatform)
        }
    }

    fn fake_backend() -> FakePrintBackend {
        FakePrintBackend {
            printers: vec![
                PrinterInfo::new("Label_Printer".to_string(), true),
                PrinterInfo::new("Office_Printer".to_string(), false),
            ],
            papers: vec![PaperInfo {
                id: "label_60x40".to_string(),
                name: "60 x 40 mm".to_string(),
                width_mm: 60.0,
                height_mm: 40.0,
            }],
        }
    }

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe {
                    env::set_var(self.key, value);
                },
                None => unsafe {
                    env::remove_var(self.key);
                },
            }
        }
    }

    fn with_config_path<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _lock = test_lock();
        let previous = env::var_os(CONFIG_PATH_OVERRIDE_ENV);
        unsafe {
            env::set_var(CONFIG_PATH_OVERRIDE_ENV, path);
        }
        let _guard = EnvGuard {
            key: CONFIG_PATH_OVERRIDE_ENV,
            previous,
        };
        f()
    }

    fn with_data_dir<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _lock = test_lock();
        let previous = env::var_os(DATA_DIR_OVERRIDE_ENV);
        unsafe {
            env::set_var(DATA_DIR_OVERRIDE_ENV, path);
        }
        let _guard = EnvGuard {
            key: DATA_DIR_OVERRIDE_ENV,
            previous,
        };
        f()
    }

    fn record_task_event(store: &TaskHistoryStore, job_id: &str, message: &str) {
        store
            .record_event(&NewTaskHistoryEvent {
                job_id,
                request_id: Some("request-1"),
                batch_id: None,
                source: TaskHistorySource::Remote,
                status: TaskHistoryStatus::Queued,
                message: Some(message),
                printer_name: Some("Office_Printer"),
                paper_name: Some("A4"),
                copies: Some(1),
                occurred_at: "2026-07-08T10:00:00Z",
            })
            .unwrap();
    }

    #[test]
    fn help_command_prints_command_list() {
        let output = run_cli_from(["help"]).unwrap();

        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("Usage: print-bridge"));
        assert!(output.stdout.contains("printer"));
        assert!(output.stdout.contains("remote"));
        assert!(output.stdout.contains("task"));
    }

    #[test]
    fn nested_help_returns_command_help_without_touching_backend() {
        let output = run_cli_with_backend(["printer", "--help"], &fake_backend()).unwrap();

        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("Usage: print-bridge printer"));
        assert!(output.stdout.contains("set-default"));
    }

    #[test]
    fn unknown_command_returns_usage_error() {
        let error = run_cli_from(["missing"]).unwrap_err();

        assert!(error
            .to_string()
            .contains("unrecognized subcommand 'missing'"));
    }

    #[test]
    fn task_list_format_is_empty_when_history_is_empty() {
        let output = format_task_list(&[]);

        assert_eq!(output, "No task history found.\n");
    }

    #[test]
    fn printer_command_lists_printers() {
        let output = run_cli_with_backend(["printer"], &fake_backend()).unwrap();

        assert!(output.stdout.contains("* Label_Printer"));
        assert!(output.stdout.contains("  Office_Printer"));
    }

    #[test]
    fn printer_detail_shows_named_printer() {
        let output = run_cli_with_backend(["printer", "Office_Printer"], &fake_backend()).unwrap();

        assert!(output.stdout.contains("name: Office_Printer"));
        assert!(output.stdout.contains("default: false"));
    }

    #[test]
    fn printer_set_default_persists_config() {
        let config_path =
            std::env::temp_dir().join(format!("print-bridge-cli-{}.json", Uuid::new_v4()));

        let output = with_config_path(&config_path, || {
            run_cli_with_backend(
                ["printer", "set-default", "Office_Printer"],
                &fake_backend(),
            )
            .unwrap()
        });

        assert!(output
            .stdout
            .contains("default printer set to Office_Printer"));

        let config = AgentConfig::load(&config_path).unwrap();
        assert_eq!(
            config.printing.default_printer.as_deref(),
            Some("Office_Printer")
        );
    }

    #[test]
    fn paper_set_rejects_zero_width() {
        let error = run_cli_with_backend(["paper", "set", "0", "40"], &fake_backend()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "paper width and height must be positive numbers"
        );
    }

    #[test]
    fn paper_set_rejects_non_finite_width() {
        for width in ["inf", "NaN"] {
            let error =
                run_cli_with_backend(["paper", "set", width, "40"], &fake_backend()).unwrap_err();

            assert_eq!(
                error.to_string(),
                "paper width and height must be positive numbers"
            );
        }
    }

    #[test]
    fn origin_add_rejects_path() {
        let error = run_cli_with_backend(
            ["origin", "add", "https://example.com/app"],
            &fake_backend(),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "invalid origin");
    }

    #[test]
    fn remote_set_interval_rejects_zero() {
        let error =
            run_cli_with_backend(["remote", "set-interval", "0"], &fake_backend()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "remote interval must be a positive integer"
        );
    }

    #[test]
    fn remote_set_url_rejects_non_http_scheme() {
        let config_path =
            std::env::temp_dir().join(format!("print-bridge-cli-{}.json", Uuid::new_v4()));

        let error = with_config_path(&config_path, || {
            run_cli_with_backend(
                ["remote", "set-url", "ftp://example.com/tasks"],
                &fake_backend(),
            )
            .unwrap_err()
        });

        assert_eq!(error.to_string(), "remote url must use http or https");
        assert!(!config_path.exists());
    }

    #[test]
    fn task_detail_reads_events_from_history_store() {
        let data_dir =
            std::env::temp_dir().join(format!("print-bridge-cli-data-{}", Uuid::new_v4()));
        fs::create_dir_all(&data_dir).unwrap();
        let history_path = data_dir.join("task_history.sqlite3");
        let store = TaskHistoryStore::open(&history_path).unwrap();
        record_task_event(&store, "job-1", "queued");
        drop(store);

        let output = with_data_dir(&data_dir, || {
            run_cli_with_backend(["task", "job-1"], &fake_backend()).unwrap()
        });

        assert!(output.stdout.contains("job_id: job-1"));
        assert!(output.stdout.contains("queued queued"));
        fs::remove_dir_all(&data_dir).unwrap();
    }

    #[test]
    fn task_clear_removes_history_store_entries() {
        let data_dir =
            std::env::temp_dir().join(format!("print-bridge-cli-data-{}", Uuid::new_v4()));
        fs::create_dir_all(&data_dir).unwrap();
        let history_path = data_dir.join("task_history.sqlite3");
        let store = TaskHistoryStore::open(&history_path).unwrap();
        record_task_event(&store, "job-1", "queued");
        drop(store);

        let output = with_data_dir(&data_dir, || {
            run_cli_with_backend(["task", "clear"], &fake_backend()).unwrap()
        });

        let store = TaskHistoryStore::open(&history_path).unwrap();
        assert_eq!(output.stdout, "task history cleared\n");
        assert!(store.recent_jobs(50).unwrap().is_empty());
        drop(store);
        fs::remove_dir_all(&data_dir).unwrap();
    }
}
