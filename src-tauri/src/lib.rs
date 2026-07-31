//! RShell Tauri 壳入口
//!
//! 占位最小实现 — 仅注册一个 `hello` 命令验证链路通。
//! 后续阶段会接入:
//!   - state.rs 持有 CommandDispatcher + EventBus
//!   - commands.rs 暴露所有 AppCommand 变体
//!   - events.rs 把 AppEvent 经 emit_to 推到前端
//!   - terminal.rs PTY/SSH 字节流经 Channel 推送

use serde::Serialize;

#[derive(Serialize)]
struct HelloResponse {
    message: String,
    backend: &'static str,
    version: &'static str,
}

/// 占位命令 — 验证 IPC 链路
#[tauri::command]
fn hello() -> HelloResponse {
    HelloResponse {
        message: "Hello from RShell backend!".to_string(),
        backend: "rust",
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Tauri 应用入口
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            tracing::info!("RShell Tauri shell started, app handle: {:?}", app.handle().package_info().name);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![hello])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}