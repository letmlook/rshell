//! 后端层错误定义

use thiserror::Error;

/// 后端层通用错误
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Service error: {0}")]
    ServiceError(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error(
        "Terminal recovery required: input outcome is uncertain; reconnect manually without replay"
    )]
    TerminalRecoveryRequired,
    #[error("Authentication error: {0}")]
    AuthError(String),
    #[error("Invalid state: {0}")]
    InvalidState(String),
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    /// 切片 1.0 引入：会话持久化失败。设计 §4.5 的硬约束 —— 任何
    /// 写磁盘失败必须阻断 create/update/delete,避免出现"内存有但磁盘无"的分裂状态。
    #[error("Storage error: {0}")]
    StorageError(String),
    /// 传输目标已存在且策略为 Fail：前端据此弹出「覆盖 / 重命名 / 取消」对话框。
    ///
    /// 单独成变体而非复用 `InvalidState`，是为了让 IPC 的 `kind` 成为稳定的
    /// 机器可读判别串——前端按 `kind == "target_exists"` 分支，不解析 message。
    #[error("Transfer target already exists: {0}")]
    TargetExists(String),
}
