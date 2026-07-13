#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod linux;
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;
mod unix_link;
#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod windows;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliIntegrationStatusKind {
    NotInstalled,
    Installed,
    InstalledSystem,
    Stale,
    Conflict,
    Unavailable,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CliIntegrationStatus {
    pub kind: CliIntegrationStatusKind,
    pub command_path: Option<String>,
    pub path_ready: bool,
}

pub fn status() -> Result<CliIntegrationStatus, String> {
    #[cfg(target_os = "macos")]
    return macos::status();
    #[cfg(target_os = "linux")]
    return linux::status();
    #[cfg(target_os = "windows")]
    return windows::status();
    #[allow(unreachable_code)]
    Ok(unsupported())
}

pub fn install() -> Result<CliIntegrationStatus, String> {
    #[cfg(target_os = "macos")]
    return macos::install();
    #[cfg(target_os = "linux")]
    return linux::install();
    #[cfg(target_os = "windows")]
    return windows::install();
    #[allow(unreachable_code)]
    Ok(unsupported())
}

pub fn uninstall() -> Result<CliIntegrationStatus, String> {
    #[cfg(target_os = "macos")]
    return macos::uninstall();
    #[cfg(target_os = "linux")]
    return linux::uninstall();
    #[cfg(target_os = "windows")]
    return windows::uninstall();
    #[allow(unreachable_code)]
    Ok(unsupported())
}

fn unsupported() -> CliIntegrationStatus {
    CliIntegrationStatus {
        kind: CliIntegrationStatusKind::Unsupported,
        command_path: None,
        path_ready: false,
    }
}
