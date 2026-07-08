// 防止 Windows release 版本额外弹出控制台窗口，请勿删除！！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if is_cli_invocation() {
        std::process::exit(print_bridge_lib::run_cli_from_env());
    }

    print_bridge_lib::run()
}

fn is_cli_invocation() -> bool {
    matches!(
        std::env::args().nth(1).as_deref(),
        Some("help" | "--help" | "-h" | "printer" | "paper" | "origin" | "remote" | "task")
    )
}
