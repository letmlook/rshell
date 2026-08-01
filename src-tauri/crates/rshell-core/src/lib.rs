//! RShell 后端层
//!
//! 核心业务逻辑，不依赖 GPUI：
//! - 终端服务（terminal）
//! - 会话管理（session）
//! - 文件传输（transfer）
//! - 安全服务（security）
//! - 脚本引擎（script）
//! - 主题管理（theme）
//! - 事件总线（event_bus）
//! - 命令分发器（command_dispatcher）

pub mod command_dispatcher;
pub mod error;
pub mod event_bus;
pub mod script;
pub mod security;
pub mod session;
pub mod terminal;
pub mod theme;
pub mod transfer;

// Re-export 主要类型
pub use command_dispatcher::CommandDispatcher;
pub use error::CoreError;
pub use event_bus::EventBus;
pub use theme::ThemeManager;

#[cfg(test)]
mod send_sync_asserts {
    use super::CommandDispatcher;

    // 设计 §1.2：D5 决议要求 Arc<CommandDispatcher> 在 Tauri State 中可共享；
    // rhai 启用 `sync` feature 后必须满足 Send + Sync。编译期断言保证不退化。
    static_assertions::assert_impl_all!(CommandDispatcher: Send, Sync);
}
