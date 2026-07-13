use super::{
    unix_link::{classify_link, LinkState},
    CliIntegrationStatus, CliIntegrationStatusKind,
};
use std::{env, path::Path, process::Command};

const COMMAND_PATH: &str = "/usr/local/bin/print-bridge";

pub(super) fn status() -> Result<CliIntegrationStatus, String> {
    let source = env::current_exe().map_err(|error| error.to_string())?;
    status_for(&source)
}

pub(super) fn install() -> Result<CliIntegrationStatus, String> {
    let source = env::current_exe().map_err(|error| error.to_string())?;
    if matches!(
        classify_link(Path::new(COMMAND_PATH), &source).map_err(|error| error.to_string())?,
        LinkState::Conflict
    ) {
        return Err(format!(
            "{COMMAND_PATH} is occupied by a non-symbolic-link entry"
        ));
    }

    run_osascript(install_script(), Some(&source))?;
    status_for(&source)
}

pub(super) fn uninstall() -> Result<CliIntegrationStatus, String> {
    let source = env::current_exe().map_err(|error| error.to_string())?;
    if classify_link(Path::new(COMMAND_PATH), &source).map_err(|error| error.to_string())?
        == LinkState::Installed
    {
        run_osascript(uninstall_script(), None)?;
    }
    status_for(&source)
}

fn status_for(source: &Path) -> Result<CliIntegrationStatus, String> {
    let kind =
        match classify_link(Path::new(COMMAND_PATH), source).map_err(|error| error.to_string())? {
            LinkState::NotInstalled => CliIntegrationStatusKind::NotInstalled,
            LinkState::Installed => CliIntegrationStatusKind::Installed,
            LinkState::Stale => CliIntegrationStatusKind::Stale,
            LinkState::Conflict => CliIntegrationStatusKind::Conflict,
        };
    Ok(CliIntegrationStatus {
        kind,
        command_path: Some(COMMAND_PATH.to_owned()),
        path_ready: true,
    })
}

fn run_osascript(script: &str, source: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new("/usr/bin/osascript");
    command.args(["-e", script]);
    if let Some(source) = source {
        command.arg(source);
    }
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if message.is_empty() {
            "macOS command-line tool operation failed".to_owned()
        } else {
            message
        })
    }
}

fn install_script() -> &'static str {
    r#"on run argv
do shell script "/bin/mkdir -p /usr/local/bin && if [ -e /usr/local/bin/print-bridge ] && [ ! -L /usr/local/bin/print-bridge ]; then exit 73; fi; /bin/ln -sfn " & quoted form of item 1 of argv & " /usr/local/bin/print-bridge" with administrator privileges
end run"#
}

fn uninstall_script() -> &'static str {
    r#"do shell script "if [ -L /usr/local/bin/print-bridge ]; then /bin/rm /usr/local/bin/print-bridge; else exit 73; fi" with administrator privileges"#
}

#[cfg(test)]
mod tests {
    use super::install_script;

    #[test]
    fn install_script_quotes_source_from_argv() {
        let script = install_script();
        assert!(script.contains("on run argv"));
        assert!(script.contains("quoted form of item 1 of argv"));
        assert!(!script.contains("/Applications/PrintBridge.app"));
    }
}
