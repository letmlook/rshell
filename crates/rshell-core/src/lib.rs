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
