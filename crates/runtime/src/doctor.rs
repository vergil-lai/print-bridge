use std::{
    env,
    net::{SocketAddr, TcpListener},
    path::Path,
};

use print_bridge_cli::{DoctorCheck, DoctorReport, DoctorStatus, ProductKind};

use crate::{
    config::AgentConfig,
    html::{
        browser::{check_browser_launch, BrowserExecutable},
        HtmlRenderError,
    },
    office::{office_candidate_statuses, OfficeCandidateStatus, OfficeFormat},
    state::AgentState,
};

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
        browser_check(check_browser_launch()),
    ];
    checks.extend(office_checks());
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

/// 把 HTML 打印使用的浏览器探测结果转换为诊断项。
fn browser_check(result: Result<BrowserExecutable, HtmlRenderError>) -> DoctorCheck {
    match result {
        Ok(browser) => check(
            "browser.available",
            DoctorStatus::Pass,
            format!("Executable found at {}.", browser.path.display()),
            None,
        ),
        Err(error) => check(
            "browser.available",
            DoctorStatus::Warn,
            error.to_string(),
            Some("Install Chrome or Chromium for HTML printing."),
        ),
    }
}

/// 返回 DOCX、XLSX、PPTX 各自的被动 Office 转换器检查。
fn office_checks() -> Vec<DoctorCheck> {
    [
        ("office.docx", OfficeFormat::Docx),
        ("office.xlsx", OfficeFormat::Xlsx),
        ("office.pptx", OfficeFormat::Pptx),
    ]
    .into_iter()
    .map(|(code, format)| office_check(code, &office_candidate_statuses(format)))
    .collect()
}

/// 汇总一个 Office 格式的候选顺序和首个被动探测结果。
fn office_check(code: &str, candidates: &[OfficeCandidateStatus]) -> DoctorCheck {
    let chain = candidates
        .iter()
        .map(|candidate| {
            format!(
                "{} ({})",
                candidate.name,
                if candidate.available {
                    "detected"
                } else {
                    "not detected"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" -> ");

    if let Some(selected) = candidates.iter().find(|candidate| candidate.available) {
        check(
            code,
            DoctorStatus::Pass,
            format!(
                "Selected {}. Candidates: {}. Runtime activation is verified when an Office print job runs.",
                selected.name, chain
            ),
            None,
        )
    } else {
        check(
            code,
            DoctorStatus::Warn,
            format!("No Office converter was detected. Candidates: {chain}."),
            Some("Install a supported Office converter for this document format."),
        )
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::browser::{BrowserExecutable, BrowserKind};
    use std::path::PathBuf;

    #[test]
    fn browser_check_reports_the_browser_selected_by_html_printing() {
        let browser = BrowserExecutable {
            kind: BrowserKind::Chrome,
            path: PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            label: "chrome",
        };

        let result = browser_check(Ok(browser));

        assert_eq!(result.status, DoctorStatus::Pass);
        assert!(result.message.contains("Google Chrome.app"));
    }

    #[test]
    fn office_check_selects_first_detected_candidate_in_order() {
        let candidates = [
            OfficeCandidateStatus {
                name: "Microsoft Word",
                available: false,
            },
            OfficeCandidateStatus {
                name: "WPS Writer",
                available: true,
            },
            OfficeCandidateStatus {
                name: "LibreOffice",
                available: true,
            },
        ];

        let result = office_check("office.docx", &candidates);

        assert_eq!(result.status, DoctorStatus::Pass);
        assert!(result.message.contains("Selected WPS Writer"));
        assert!(result.message.contains(
            "Microsoft Word (not detected) -> WPS Writer (detected) -> LibreOffice (detected)"
        ));
    }

    #[test]
    fn office_check_warns_when_no_candidate_is_detected() {
        let candidates = [OfficeCandidateStatus {
            name: "LibreOffice",
            available: false,
        }];

        let result = office_check("office.xlsx", &candidates);

        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("No Office converter was detected"));
    }
}
