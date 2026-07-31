// RShell - Tauri 2.0 主入口
// 阻止额外窗口在 Windows 上创建(默认行为)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rshell_lib::run();
}