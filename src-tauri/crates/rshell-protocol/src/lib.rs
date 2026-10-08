//! RShell 协议层
//!
//! 实现各种远程连接协议：
//! - SSH（含 SFTP）
//! - Telnet
//! - Serial

pub mod serial;
pub mod ssh;
pub mod telnet;
/// R2-T2：传输抽象与本地 sink
pub mod transfer;

use thiserror::Error;

/// 协议层通用错误
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    /// Host-key verification was rejected, cancelled, or expired. This is
    /// distinct from a transport failure so the UI can offer host-key
    /// recovery instead of a generic network retry.
    #[error("Host key mismatch: {0}")]
    HostKeyMismatch(String),
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("Connection closed")]
    ConnectionClosed,
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    #[error(
        "Terminal recovery required: input outcome is uncertain; reconnect manually without replay"
    )]
    TerminalRecoveryRequired,
    #[error("Timeout")]
    Timeout,
    #[error("Transfer cancelled")]
    TransferCancelled,
    /// 目标已存在且策略为「发现冲突即失败」。在**创建/截断之前**抛出，
    /// 保证既有文件不被破坏。
    #[error("Transfer target already exists: {0}")]
    TransferConflict(String),
    /// R2-T2：提交前重新检查目标，发现冲突且用户策略不允许覆盖。
    /// 与 `TransferConflict` 不同：发生在「临时文件已写完、即将替换之前」，
    /// 用于拦截预检之后、提交之前这段窗口里新出现的同名文件。
    #[error("Transfer target appeared before commit: {0}")]
    TransferLateConflict(String),
    /// R2-T2：临时文件已写完，但最终提交（rename / posix-rename）失败。
    /// 旧目标保持原样，临时文件可能残留，调用方应报告确切路径与原因。
    #[error("Transfer commit failed for {path}: {reason}")]
    TransferCommitFailed { path: String, reason: String },
    /// R2-T2：临时文件已写完，但提交失败后清理临时文件也失败。
    /// 调用方应把残留路径原样告诉用户，不假装清理成功。
    #[error("Transfer cleanup failed for {path}: {reason}")]
    TransferCleanupFailed { path: String, reason: String },
}

/// 连接 trait（所有协议的统一抽象）
// async_trait 宏为每个 async 方法生成的 #[must_use] 与返回值
// Pin<Box<dyn Future>> 自带的 #[must_use] 重复，触发 clippy::double_must_use。
// 该 lint 报在宏展开上，只能在此处定点豁免。
#[allow(clippy::double_must_use)]
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
