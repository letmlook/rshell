//! 终端模块 —— 切片 2.1 清理后
//!
//! 设计 §2.2：alacritty 缓冲已删除,本模块仅保留 TerminalService
//! （生命周期 + 尺寸记录）。`buffer` 子模块已移除。

pub mod service;