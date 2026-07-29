//! RShell 基础设施层
//!
//! 提供底层能力抽象：
//! - 加密（crypto）
//! - 持久化存储（storage）
//! - PTY 抽象（pty）

pub mod crypto;
pub mod pty;
pub mod storage;
