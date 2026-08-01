//! RShell API 边界层
//!
//! 定义前端与后端之间的通信协议：
//! - `AppCommand`: 前端发往后端的命令意图
//! - `AppEvent`: 后端发往前端的状态事件
//! - `CommandOutcome`: 读命令返回值类型（设计 §3.2）
//!
//! 约束：零运行时依赖（除 serde / uuid）—— 设计 §1.3
//! 切片 2.3 状态：ts-rs 12 已加入 workspace 依赖,但全量 `#[derive(TS)]` 需要
//! `types.rs` 中所有公共结构体同步 derive —— 按功能域逐项导出是切片 3+ 的工作量。
//! 导出路径约定:`#[ts(export_to = "../../../src/ipc/generated.ts")]`,
//! CI 加 `git diff --exit-code src/ipc/generated.ts` 拦截漂移（设计 §3.6）。

pub mod commands;
pub mod events;
pub mod outcome;
pub mod types;

// Re-export 主要类型
pub use commands::AppCommand;
pub use events::AppEvent;
pub use outcome::CommandOutcome;
pub use types::*;
