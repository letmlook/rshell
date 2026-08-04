//! 终端模块 —— 切片 2.1 清理后
//!
//! 设计 §2.2:alacritty 缓冲已删除,本模块仅保留 TerminalService
//! (生命周期 + 尺寸记录)。`buffer` 子模块已移除。
//!
//! 2026-08-04:`channels` 子模块从壳层提升到本模块,让 SessionService 的
//! SSH recv 循环可以直接 push 字节给前端 xterm —— 见 channels.rs 顶部注释。

pub mod channels;
pub mod service;

pub use channels::{SharedTerminalChannels, TerminalChannels};