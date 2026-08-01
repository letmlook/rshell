//! RShell 协议层
//!
//! 实现各种远程连接协议：
//! - SSH（含 SFTP）
//! - Telnet
//! - Serial
//! - RDP

pub mod rdp;
pub mod serial;
pub mod ssh;
pub mod telnet;

use thiserror::Error;

/// 协议层通用错误
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("Connection closed")]
    ConnectionClosed,
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    #[error("Timeout")]
    Timeout,
}

/// 连接 trait（所有协议的统一抽象）
#[async_trait::async_trait]
pub trait Connection: Send + Sync {
    /// 连接到远程主机
    async fn connect(&mut self) -> Result<(), ProtocolError>;
    /// 断开连接
    async fn disconnect(&mut self) -> Result<(), ProtocolError>;
    /// 发送数据
    async fn send(&mut self, data: &[u8]) -> Result<(), ProtocolError>;
    /// 接收数据
    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, ProtocolError>;
    /// 调整终端大小
    async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), ProtocolError>;
}
