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
    /// The metadata says a credential is required, but the secure store has
    /// no entry. The UI can offer the existing credential editor.
    #[error("Credential missing: {0}")]
    CredentialMissing(String),
    /// The secure store could not be accessed (for example Keychain/DPAPI/
    /// Secret Service refused access). Never fall back to plaintext.
    #[error("Credential storage inaccessible: {0}")]
    CredentialInaccessible(String),
    /// A credential write was refused. No connection may be started from the
    /// failed save, and the editor must keep the user's input.
    #[error("Credential save failed: {0}")]
    CredentialSaveFailed(String),
    /// A legacy plaintext credential migration could not be committed. The
    /// original safe cause is retained so the explicit retry-load path can be
    /// used after access is repaired.
    #[error("Credential migration failed: {0}")]
    CredentialMigrationFailed(String),
    /// SSH host-key verification ended without an accepted, persisted decision.
    #[error("Host key mismatch: {0}")]
    HostKeyMismatch(String),
    /// Permanent trust could not be persisted; the handshake remains rejected.
    #[error("Host key trust persistence failed: {0}")]
    HostKeyTrustPersistenceFailed(String),
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
