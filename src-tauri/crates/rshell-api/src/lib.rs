//! RShell API 边界层
//!
//! 定义前端与后端之间的通信协议：
//! - `AppCommand`: 前端发往后端的命令意图
//! - `AppEvent`: 后端发往前端的状态事件
//!
//! 约束：零运行时依赖，仅包含纯数据类型

pub mod commands;
pub mod events;
pub mod types;

// Re-export 主要类型
pub use commands::AppCommand;
pub use events::AppEvent;
pub use types::*;
