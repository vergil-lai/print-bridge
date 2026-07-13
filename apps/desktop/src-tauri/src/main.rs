// 防止 Windows release 版本额外弹出控制台窗口，请勿删除！！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(not(target_os = "windows"))]
    if is_cli_invocation() {
        std::process::exit(print_bridge_lib::run_cli_from_env());
    }

    print_bridge_lib::run()
}

#[cfg(not(target_os = "windows"))]
fn is_cli_invocation() -> bool {
    let mut args = std::env::args();
    should_run_cli(args.next().as_deref(), args.next().as_deref())
}

#[cfg(not(target_os = "windows"))]
fn is_cli_command(command: Option<&str>) -> bool {
    command.is_some()
}

/// 根据调用名和首个参数判断是否应进入 CLI。
#[cfg(not(target_os = "windows"))]
fn should_run_cli(program: Option<&str>, command: Option<&str>) -> bool {
    is_cli_command(command)
        || program
            .and_then(|value| std::path::Path::new(value).file_name())
            .and_then(|value| value.to_str())
            == Some("print-bridge")
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::*;

    #[test]
    fn serve_is_routed_to_cli_for_an_unknown_command_error() {
        assert!(is_cli_command(Some("serve")));
    }

    #[test]
    fn unknown_command_is_routed_to_cli_error() {
        assert!(is_cli_command(Some("missing")));
    }

    #[test]
    fn print_bridge_command_without_arguments_is_routed_to_cli_help() {
        assert!(should_run_cli(Some("/usr/local/bin/print-bridge"), None));
    }
}
