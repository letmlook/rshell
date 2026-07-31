//! PTY 抽象模块
//!
//! 提供跨平台的伪终端抽象

pub mod unix;
pub mod windows;

use thiserror::Error;

/// PTY 错误
#[derive(Debug, Error)]
pub enum PtyError {
    #[error("PTY creation failed: {0}")]
    CreationFailed(String),
    #[error("PTY operation failed: {0}")]
    OperationFailed(String),
}

/// PTY trait（跨平台抽象）
pub trait Pty: Send + Sync {
    /// 读取数据
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    /// 写入数据
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize>;
    /// 调整大小
    fn resize(&mut self, rows: u16, cols: u16) -> std::io::Result<()>;
}
