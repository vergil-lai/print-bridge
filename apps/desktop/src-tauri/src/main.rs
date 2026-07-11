// 防止 Windows release 版本额外弹出控制台窗口，请勿删除！！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if is_cli_invocation() {
        std::process::exit(print_bridge_lib::run_cli_from_env());
    }

    print_bridge_lib::run()
}

fn is_cli_invocation() -> bool {
    is_cli_command(std::env::args().nth(1).as_deref())
}

fn is_cli_command(command: Option<&str>) -> bool {
    command.is_some()
}

#[cfg(test)]
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
}
