//! Tauri 壳的全局状态。
//!
//! 设计 §1.1 / §1.2：`AppState` 持有完整 `CommandDispatcher` + EventBus 桥 +
//! 终端字节双态 sink 注册表。`#[tauri::command]` 薄壳通过 `State<'_, AppState>` 读取。
//!
//! `CommandDispatcher` 内仅持 `Arc<T>` 与 rhai `sync` feature 开启后的 `rhai::Engine`
//! —— 已在 `rshell-core/src/lib.rs` 的 `assert_impl_all!` 编译期断言为 `Send + Sync`。

use std::sync::Arc;

use rshell_core::CommandDispatcher;

use crate::terminal::TerminalChannels;

pub struct AppState {
    pub dispatcher: Arc<CommandDispatcher>,
    /// 终端字节双态 sink：`attach_terminal` 登记通道，recv 循环 `push` 字节，
    /// 会话删除与标签关闭分别按会话 / 按 `(session, terminal)` 粒度清理
    /// （见 lib.rs 的删除回调与 commands.rs 的 close_terminal）。
    pub terminal_channels: Arc<TerminalChannels>,
}
