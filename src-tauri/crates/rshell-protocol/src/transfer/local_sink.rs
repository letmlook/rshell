//! R2-T2：`TransferSink` 的本地实现 —— 把 `tokio::fs` 拼成 staging 接口。
//!
//! 上传侧：源是本地文件 → staging 也是本地文件 → 提交是本地 rename。
//! 下载侧：源是远端 → staging 在本地 → 提交也是本地 rename。
//!
//! 因此 `LocalSink` 在两个方向都会用到：下载侧的 staging 与提交、上传侧
//! 也可能用（虽然当前默认 staging 在远端，见 `TransferService` 的实现）。
//!
//! 行为约束：
//! - `exclusive_open_write`：用 `OpenOptions::create_new(true)`，等价于
//!   POSIX `O_CREAT | O_EXCL`；存在即拒绝，与 `SftpClient::open_exclusive` 同语义。
//! - `safe_commit`：本地的 `tokio::fs::rename` 在 macOS / Linux / Windows
//!   上都是「原子替换」（参见 `std::fs::rename` 文档），所以一律返回
//!   `CommitStrategy::StandardRename`；**不**撒谎说成 `PosixRename`，
//!   但本地 rename 实际上比 SFTP 标准 rename 更靠谱。
//! - `try_remove`：「不存在」一律视为成功，与 SFTP 实现对齐。

use super::super::ssh::sftp::{CommitOutcome, CommitStrategy, SftpCapabilities, TransferSink};
use crate::ProtocolError;
use std::path::Path;
use tokio::io::{AsyncRead, AsyncWrite};

/// R2-T2：本地 `TransferSink`。该类型零状态，可任意克隆、共享。
#[derive(Clone, Default)]
pub struct LocalSink;

impl LocalSink {
    pub fn new() -> Self {
        Self
    }
}

#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
impl TransferSink for LocalSink {
    async fn exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    async fn exclusive_open_write(
        &self,
        path: &str,
    ) -> Result<Box<dyn AsyncWrite + Unpin + Send>, ProtocolError> {
        // create_new(true) = 仅在文件不存在时创建；等价于 O_CREAT|O_EXCL。
        // 已有同名临时文件即拒绝——上层应据此报告「同 UUID 撞同一目标」。
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await
            .map_err(|e| {
                ProtocolError::ProtocolError(format!(
                    "Failed to create local temp file {path}: {e}"
                ))
            })?;
        Ok(Box::new(file))
    }

    async fn open_read(
        &self,
        path: &str,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, ProtocolError> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to open {path}: {e}")))?;
        Ok(Box::new(file))
    }

    async fn safe_commit(
        &self,
        from: &str,
        to: &str,
        _caps: SftpCapabilities,
    ) -> Result<CommitOutcome, ProtocolError> {
        // 本地 `std::fs::rename` 在所有目标平台（macOS / Linux / Windows）
        // 上均为「原子替换」语义，且成功即可生效——比 SFTP 标准
        // `SSH_FXP_RENAME` 更靠谱。但 capability 探测接口仍走同一路径，
        // 因此继续返回 `StandardRename`，由「能力来源」区分。
        tokio::fs::rename(from, to)
            .await
            .map_err(|e| ProtocolError::TransferCommitFailed {
                path: to.to_string(),
                reason: format!("local rename failed: {e}"),
            })?;
        Ok(CommitOutcome {
            strategy: CommitStrategy::StandardRename,
        })
    }

    async fn try_remove(&self, path: &str) -> Result<(), ProtocolError> {
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ProtocolError::TransferCleanupFailed {
                path: path.to_string(),
                reason: format!("local cleanup failed: {e}"),
            }),
        }
    }

    fn is_remote(&self) -> bool {
        false
    }
}
