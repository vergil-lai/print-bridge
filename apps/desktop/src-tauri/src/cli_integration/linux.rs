use super::{
    unix_link::{classify_link, LinkState},
    CliIntegrationStatus, CliIntegrationStatusKind,
};
use std::{
    env, fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum LinuxInstallKind {
    System { command: PathBuf },
    AppImage { source: PathBuf, command: PathBuf },
    Unavailable,
}

pub(super) fn status() -> Result<CliIntegrationStatus, String> {
    let kind = current_install_kind()?;
    status_for(&kind)
}

pub(super) fn install() -> Result<CliIntegrationStatus, String> {
    let kind = current_install_kind()?;
    if let LinuxInstallKind::AppImage { source, command } = &kind {
        match classify_link(command, source).map_err(|error| error.to_string())? {
            LinkState::Conflict => {
                return Err(format!(
                    "{} is occupied by a non-symbolic-link entry",
                    command.display()
                ));
            }
            LinkState::Installed => return status_for(&kind),
            LinkState::Stale => fs::remove_file(command).map_err(|error| error.to_string())?,
            LinkState::NotInstalled => {}
        }
        fs::create_dir_all(command.parent().ok_or("invalid command path")?)
            .map_err(|error| error.to_string())?;
        symlink(source, command).map_err(|error| error.to_string())?;
    }
    status_for(&kind)
}

pub(super) fn uninstall() -> Result<CliIntegrationStatus, String> {
    let kind = current_install_kind()?;
    if let LinuxInstallKind::AppImage { source, command } = &kind {
        if classify_link(command, source).map_err(|error| error.to_string())?
            == LinkState::Installed
        {
            fs::remove_file(command).map_err(|error| error.to_string())?;
        }
    }
    status_for(&kind)
}

fn current_install_kind() -> Result<LinuxInstallKind, String> {
    let current_exe = env::current_exe().map_err(|error| error.to_string())?;
    let appimage = env::var_os("APPIMAGE").map(PathBuf::from);
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    Ok(detect_install_kind(
        &current_exe,
        appimage.as_deref(),
        &home,
    ))
}

fn detect_install_kind(
    current_exe: &Path,
    appimage: Option<&Path>,
    home: &Path,
) -> LinuxInstallKind {
    if let Some(source) = appimage {
        return LinuxInstallKind::AppImage {
            source: source.to_path_buf(),
            command: home.join(".local/bin/print-bridge"),
        };
    }
    if current_exe == Path::new("/usr/bin/print-bridge") {
        return LinuxInstallKind::System {
            command: current_exe.to_path_buf(),
        };
    }
    LinuxInstallKind::Unavailable
}

fn status_for(kind: &LinuxInstallKind) -> Result<CliIntegrationStatus, String> {
    match kind {
        LinuxInstallKind::System { command } => Ok(CliIntegrationStatus {
            kind: CliIntegrationStatusKind::InstalledSystem,
            command_path: Some(command.display().to_string()),
            path_ready: true,
        }),
        LinuxInstallKind::AppImage { source, command } => {
            let kind = match classify_link(command, source).map_err(|error| error.to_string())? {
                LinkState::NotInstalled => CliIntegrationStatusKind::NotInstalled,
                LinkState::Installed => CliIntegrationStatusKind::Installed,
                LinkState::Stale => CliIntegrationStatusKind::Stale,
                LinkState::Conflict => CliIntegrationStatusKind::Conflict,
            };
            let parent = command.parent().ok_or("invalid command path")?;
            let path_ready = env::var_os("PATH")
                .map(|value| env::split_paths(&value).any(|entry| entry == parent))
                .unwrap_or(false);
            Ok(CliIntegrationStatus {
                kind,
                command_path: Some(command.display().to_string()),
                path_ready,
            })
        }
        LinuxInstallKind::Unavailable => Ok(CliIntegrationStatus {
            kind: CliIntegrationStatusKind::Unavailable,
            command_path: None,
            path_ready: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_install_kind, LinuxInstallKind};
    use std::path::{Path, PathBuf};

    #[test]
    fn packaged_binary_is_system_installed() {
        assert_eq!(
            detect_install_kind(
                Path::new("/usr/bin/print-bridge"),
                None,
                Path::new("/home/user")
            ),
            LinuxInstallKind::System {
                command: PathBuf::from("/usr/bin/print-bridge")
            }
        );
    }

    #[test]
    fn appimage_uses_original_file_as_link_source() {
        assert_eq!(
            detect_install_kind(
                Path::new("/tmp/.mount_PrintBridge/usr/bin/print-bridge"),
                Some(Path::new("/home/user/PrintBridge.AppImage")),
                Path::new("/home/user")
            ),
            LinuxInstallKind::AppImage {
                source: PathBuf::from("/home/user/PrintBridge.AppImage"),
                command: PathBuf::from("/home/user/.local/bin/print-bridge")
            }
        );
    }

    #[test]
    fn unknown_portable_binary_is_unavailable() {
        assert_eq!(
            detect_install_kind(
                Path::new("/opt/PrintBridge/print-bridge"),
                None,
                Path::new("/home/user")
            ),
            LinuxInstallKind::Unavailable
        );
    }
}
