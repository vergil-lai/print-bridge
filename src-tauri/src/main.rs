// 防止 Windows release 版本额外弹出控制台窗口，请勿删除！！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    print_bridge_lib::run()
}
