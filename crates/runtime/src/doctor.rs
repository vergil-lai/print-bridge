use std::{
    env,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
};

use print_bridge_cli::{DoctorCheck, DoctorReport, DoctorStatus, ProductKind};

use crate::{config::AgentConfig, state::AgentState};

/// 执行不产生业务副作用的本地环境检查。
pub async fn run_doctor(
    state: &AgentState,
    listen_addr: SocketAddr,
    product: ProductKind,
) -> DoctorReport {
    let config = state.config.read().await.clone();
    let mut checks = vec![
        config_check(state.config_path.as_deref()),
        directory_check(state.config_path.as_deref()),
        agent_check(listen_addr),
        port_check(&config, listen_addr),
        printer_check(state),
        executable_check(
            "browser.available",
            browser_candidates(),
            "Install Chrome or Chromium for HTML printing.",
        ),
        executable_check("office.available", office_candidates(), office_suggestion()),
    ];
    if product == ProductKind::Headless {
        checks.push(systemd_check(state.config_path.as_deref()));
    }
    checks.push(remote_check(&config));
    DoctorReport::new(checks)
}

fn config_check(path: Option<&Path>) -> DoctorCheck {
    match path {
        Some(path) => match AgentConfig::load(path) {
            Ok(_) => check(
                "config.valid",
                DoctorStatus::Pass,
                "Configuration is readable and valid.",
                None,
            ),
            Err(error) => check(
                "config.valid",
                DoctorStatus::Fail,
                format!("Configuration is invalid: {error}"),
                Some("Fix the configuration file and run doctor again."),
            ),
        },
        None => check(
            "config.valid",
            DoctorStatus::Warn,
            "No persistent configuration path is configured.",
            None,
        ),
    }
}

fn directory_check(path: Option<&Path>) -> DoctorCheck {
    let Some(directory) = path.and_then(Path::parent) else {
        return check(
            "data.directory",
            DoctorStatus::Warn,
            "Data directory is unknown.",
            None,
        );
    };
    match std::fs::metadata(directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.permissions().readonly() => check(
            "data.directory",
            DoctorStatus::Pass,
            "Data directory is accessible.",
            None,
        ),
        Ok(_) => check(
            "data.directory",
            DoctorStatus::Fail,
            "Data directory is not writable.",
            Some("Grant the PrintBridge process write permission."),
        ),
        Err(error) => check(
            "data.directory",
            DoctorStatus::Fail,
            format!("Data directory is inaccessible: {error}"),
            Some("Create the directory and grant the PrintBridge process access."),
        ),
    }
}

fn agent_check(addr: SocketAddr) -> DoctorCheck {
    if addr.port() == 0 {
        check(
            "agent.ipc",
            DoctorStatus::Warn,
            "The Agent is not running or local IPC is unavailable.",
            Some("Start the PrintBridge Agent when online-only commands are needed."),
        )
    } else {
        check(
            "agent.ipc",
            DoctorStatus::Pass,
            format!("The Agent is reachable at {addr}."),
            None,
        )
    }
}

fn port_check(config: &AgentConfig, addr: SocketAddr) -> DoctorCheck {
    if addr.port() == config.service.port {
        return check(
            "service.port",
            DoctorStatus::Pass,
            format!("Service port {} is active.", config.service.port),
            None,
        );
    }
    match TcpListener::bind(("0.0.0.0", config.service.port)) {
        Ok(listener) => {
            drop(listener);
            check(
                "service.port",
                DoctorStatus::Pass,
                format!("Service port {} is available.", config.service.port),
                None,
            )
        }
        Err(_) => check(
            "service.port",
            DoctorStatus::Warn,
            format!(
                "Service port {} is occupied while this Agent is offline.",
                config.service.port
            ),
            Some("Check which process owns the configured port."),
        ),
    }
}

fn printer_check(state: &AgentState) -> DoctorCheck {
    match state.printing.list_printers() {
        Ok(printers) if printers.is_empty() => check(
            "printing.printers",
            DoctorStatus::Warn,
            "No printers were found.",
            Some("Install a printer and verify the platform print service."),
        ),
        Ok(printers) => check(
            "printing.printers",
            DoctorStatus::Pass,
            format!("{} printer(s) found.", printers.len()),
            None,
        ),
        Err(error) => check(
            "printing.printers",
            DoctorStatus::Fail,
            format!("Printer enumeration failed: {error}"),
            Some("Verify the platform printing service and permissions."),
        ),
    }
}

fn executable_check(code: &str, candidates: &[&str], suggestion: &str) -> DoctorCheck {
    if let Some(path) = find_executable(candidates) {
        check(
            code,
            DoctorStatus::Pass,
            format!("Executable found at {}.", path.display()),
            None,
        )
    } else {
        check(
            code,
            DoctorStatus::Warn,
            format!("None of {} were found.", candidates.join(", ")),
            Some(suggestion),
        )
    }
}

fn find_executable(candidates: &[&str]) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .flat_map(|directory| candidates.iter().map(move |name| directory.join(name)))
        .find(|path| path.is_file())
}

#[cfg(target_os = "windows")]
fn browser_candidates() -> &'static [&'static str] {
    &["chrome.exe", "msedge.exe"]
}
#[cfg(target_os = "macos")]
fn browser_candidates() -> &'static [&'static str] {
    &["Google Chrome", "Chromium"]
}
#[cfg(all(unix, not(target_os = "macos")))]
fn browser_candidates() -> &'static [&'static str] {
    &["google-chrome", "chromium", "chromium-browser"]
}

#[cfg(target_os = "windows")]
fn office_candidates() -> &'static [&'static str] {
    &["WINWORD.EXE", "EXCEL.EXE", "POWERPNT.EXE"]
}
#[cfg(not(target_os = "windows"))]
fn office_candidates() -> &'static [&'static str] {
    &["libreoffice", "soffice"]
}

#[cfg(target_os = "windows")]
fn office_suggestion() -> &'static str {
    "Install Microsoft Office for Office document printing."
}
#[cfg(not(target_os = "windows"))]
fn office_suggestion() -> &'static str {
    "Install LibreOffice for Office document printing."
}

fn systemd_check(path: Option<&Path>) -> DoctorCheck {
    if env::var_os("INVOCATION_ID").is_some() && path.is_some() {
        check(
            "headless.systemd",
            DoctorStatus::Pass,
            "Headless Agent is running under systemd with persistent paths.",
            None,
        )
    } else {
        check(
            "headless.systemd",
            DoctorStatus::Warn,
            "systemd invocation metadata was not detected.",
            Some("Run the packaged print-bridge system service for production use."),
        )
    }
}

fn remote_check(config: &AgentConfig) -> DoctorCheck {
    if !config.remote.enabled {
        return check(
            "remote.configuration",
            DoctorStatus::Pass,
            "Remote tasks are disabled.",
            None,
        );
    }
    let complete = config.remote.endpoint_url.is_some()
        && config.remote.bearer_token.is_some()
        && config.remote.device_id.is_some();
    if complete {
        check(
            "remote.configuration",
            DoctorStatus::Pass,
            "Remote task configuration is complete; no network request was made.",
            None,
        )
    } else {
        check(
            "remote.configuration",
            DoctorStatus::Warn,
            "Remote tasks are enabled but required settings are incomplete.",
            Some("Configure the URL, bearer token, and device ID."),
        )
    }
}

fn check(
    code: impl Into<String>,
    status: DoctorStatus,
    message: impl Into<String>,
    suggestion: Option<&str>,
) -> DoctorCheck {
    DoctorCheck {
        code: code.into(),
        status,
        message: message.into(),
        suggestion: suggestion.map(str::to_string),
    }
}
