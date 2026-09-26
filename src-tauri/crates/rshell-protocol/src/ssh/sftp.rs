//! SFTP 文件传输客户端
//!
//! 基于 russh-sftp 实现 SFTP 文件传输功能：
//! - 远程目录浏览
//! - 文件上传/下载
//! - 文件元数据查询

use crate::ProtocolError;
use rshell_api::types::{FilePermissions, FileType, RemoteFileEntry};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info};

/// 单次读写分块大小（64 KiB）
const CHUNK_SIZE: usize = 64 * 1024;

/// SFTP 客户端
///
/// 封装 russh_sftp::client::SftpSession，提供高层文件操作接口。
pub struct SftpClient {
    session: russh_sftp::client::SftpSession,
}

impl SftpClient {
    /// 从 SSH 通道创建 SFTP 客户端
    ///
    /// `channel` 必须已经请求了 sftp 子系统。
    /// 调用 `channel.into_stream()` 将其转换为 AsyncRead + AsyncWrite 流。
    pub async fn new(channel: russh::Channel<russh::client::Msg>) -> Result<Self, ProtocolError> {
        let stream = channel.into_stream();
        let session = russh_sftp::client::SftpSession::new(stream)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("SFTP init failed: {}", e)))?;

        info!("SFTP session initialized");
        Ok(Self { session })
    }

    /// 列出远程目录内容
    pub async fn list_dir(&self, path: &str) -> Result<Vec<RemoteFileEntry>, ProtocolError> {
        debug!(path = %path, "Listing remote directory");

        let read_dir = self
            .session
            .read_dir(path)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("read_dir failed: {}", e)))?;

        let entries: Vec<RemoteFileEntry> = read_dir
            .map(|entry| {
                let metadata = entry.metadata();
                let file_type = match metadata.file_type() {
                    russh_sftp::protocol::FileType::Dir => FileType::Directory,
                    russh_sftp::protocol::FileType::Symlink => FileType::Symlink,
                    russh_sftp::protocol::FileType::File => FileType::File,
                    _ => FileType::Other,
                };

                let perms = metadata.permissions();
                let permissions = FilePermissions {
                    owner_read: perms.owner_read,
                    owner_write: perms.owner_write,
                    owner_execute: perms.owner_exec,
                    group_read: perms.group_read,
                    group_write: perms.group_write,
                    group_execute: perms.group_exec,
                    other_read: perms.other_read,
                    other_write: perms.other_write,
                    other_execute: perms.other_exec,
                };

                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_default();

                RemoteFileEntry {
                    name: entry.file_name(),
                    file_type,
                    size: metadata.len(),
                    permissions,
                    owner: metadata.user.clone().unwrap_or_default(),
                    group: metadata.group.clone().unwrap_or_default(),
                    modified,
                }
            })
            .collect();

        debug!(path = %path, count = entries.len(), "Directory listed");
        Ok(entries)
    }

    /// 上传本地文件到远程（分块写入，边传边回调进度）
    pub async fn upload<F>(
        &self,
        local: &PathBuf,
        remote: &str,
        mut progress: F,
    ) -> Result<u64, ProtocolError>
    where
        F: FnMut(u64, u64),
    {
        info!(local = %local.display(), remote = %remote, "Uploading file");

        let total = tokio::fs::metadata(local)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to stat local file: {}", e)))?
            .len();

        let mut source = tokio::fs::File::open(local).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to open local file: {}", e))
        })?;

        let mut target = self.session.create(remote).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to create remote file: {}", e))
        })?;

        let copied =
            copy_with_progress(&mut source, &mut target, total, CHUNK_SIZE, &mut progress).await?;
        drop(target);

        info!(remote = %remote, bytes = copied, "Upload completed");
        Ok(copied)
    }

    /// 下载远程文件到本地（分块写入，边传边回调进度）
    pub async fn download<F>(
        &self,
        remote: &str,
        local: &PathBuf,
        mut progress: F,
    ) -> Result<u64, ProtocolError>
    where
        F: FnMut(u64, u64),
    {
        info!(remote = %remote, local = %local.display(), "Downloading file");

        let total = self
            .session
            .metadata(remote)
            .await
            .map_err(|e| {
                ProtocolError::ProtocolError(format!("Failed to stat remote file: {}", e))
            })?
            .len();

        let mut source = self.session.open(remote).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to open remote file: {}", e))
        })?;

        // 确保本地目录存在
        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                ProtocolError::ProtocolError(format!("Failed to create local dir: {}", e))
            })?;
        }
        let mut target = tokio::fs::File::create(local).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to create local file: {}", e))
        })?;

        let copied =
            copy_with_progress(&mut source, &mut target, total, CHUNK_SIZE, &mut progress).await?;

        info!(remote = %remote, bytes = copied, "Download completed");
        Ok(copied)
    }

    /// 获取远程文件元数据
    pub async fn metadata(&self, path: &str) -> Result<RemoteFileEntry, ProtocolError> {
        let metadata = self
            .session
            .metadata(path)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("stat failed: {}", e)))?;

        let file_type = match metadata.file_type() {
            russh_sftp::protocol::FileType::Dir => FileType::Directory,
            russh_sftp::protocol::FileType::Symlink => FileType::Symlink,
            russh_sftp::protocol::FileType::File => FileType::File,
            _ => FileType::Other,
        };

        let perms = metadata.permissions();
        let permissions = FilePermissions {
            owner_read: perms.owner_read,
            owner_write: perms.owner_write,
            owner_execute: perms.owner_exec,
            group_read: perms.group_read,
            group_write: perms.group_write,
            group_execute: perms.group_exec,
            other_read: perms.other_read,
            other_write: perms.other_write,
            other_execute: perms.other_exec,
        };

        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default();

        Ok(RemoteFileEntry {
            name,
            file_type,
            size: metadata.len(),
            permissions,
            owner: metadata.user.clone().unwrap_or_default(),
            group: metadata.group.clone().unwrap_or_default(),
            modified,
        })
    }

    /// 创建远程目录
    pub async fn create_dir(&self, path: &str) -> Result<(), ProtocolError> {
        self.session
            .create_dir(path)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("mkdir failed: {}", e)))?;
        Ok(())
    }

    /// 删除远程文件
    pub async fn remove_file(&self, path: &str) -> Result<(), ProtocolError> {
        self.session
            .remove_file(path)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("remove failed: {}", e)))?;
        Ok(())
    }

    /// 删除远程目录
    pub async fn remove_dir(&self, path: &str) -> Result<(), ProtocolError> {
        self.session
            .remove_dir(path)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("rmdir failed: {}", e)))?;
        Ok(())
    }

    /// 重命名远程文件/目录
    pub async fn rename(&self, old_path: &str, new_path: &str) -> Result<(), ProtocolError> {
        self.session
            .rename(old_path, new_path)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("rename failed: {}", e)))?;
        Ok(())
    }

    /// 获取远程绝对路径
    pub async fn canonicalize(&self, path: &str) -> Result<String, ProtocolError> {
        self.session
            .canonicalize(path)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("canonicalize failed: {}", e)))
    }

    /// 关闭 SFTP 会话
    pub async fn close(&self) -> Result<(), ProtocolError> {
        self.session
            .close()
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("close failed: {}", e)))?;
        Ok(())
    }
}

/// 分块拷贝，进度变化时回调 `(bytes_done, total)`。
///
/// 开始前先回调一次 `(0, total)`，返回前保证发出终帧；
/// 声明的 `total` 与实际拷贝字节数不一致（源在传输期间被改写）时报错，
/// 避免远端留下被截断的文件。
async fn copy_with_progress<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    total: u64,
    chunk_size: usize,
    progress: &mut F,
) -> Result<u64, ProtocolError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
    F: FnMut(u64, u64),
{
    progress(0, total);

    let mut buf = vec![0u8; chunk_size.max(1)];
    let mut done: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("read failed during copy: {e}")))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("write failed during copy: {e}")))?;
        done += n as u64;
        progress(done, total);
    }
    writer
        .flush()
        .await
        .map_err(|e| ProtocolError::ProtocolError(format!("flush failed during copy: {e}")))?;

    if total > 0 && done != total {
        return Err(ProtocolError::ProtocolError(format!(
            "source changed during transfer: expected {total} bytes, copied {done}"
        )));
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn copy_reports_monotonic_progress_and_exact_final_total() {
        let payload = vec![7u8; 1000];
        let mut reader: &[u8] = payload.as_slice();
        let mut writer: Vec<u8> = Vec::new();
        let mut calls: Vec<(u64, u64)> = Vec::new();

        let copied = copy_with_progress(&mut reader, &mut writer, 1000, 128, &mut |done, total| {
            calls.push((done, total));
        })
        .await
        .unwrap();

        assert_eq!(copied, 1000);
        assert_eq!(writer.len(), 1000);
        assert!(calls.len() > 1, "1000 字节 / 128 分块应产生多次回调");
        assert_eq!(calls[0], (0, 1000), "开始前应先报一次 (0, total)");
        let mut prev = 0u64;
        for (done, total) in &calls {
            assert_eq!(*total, 1000);
            assert!(*done >= prev, "进度不允许回退");
            prev = *done;
        }
        assert_eq!(calls.last().copied(), Some((1000, 1000)));
    }

    #[tokio::test]
    async fn copy_accepts_unknown_total_but_still_reports_bytes() {
        let payload = vec![1u8; 300];
        let mut reader: &[u8] = payload.as_slice();
        let mut writer: Vec<u8> = Vec::new();
        let mut last = (0u64, 0u64);

        let copied = copy_with_progress(&mut reader, &mut writer, 0, 64, &mut |d, t| last = (d, t))
            .await
            .unwrap();

        assert_eq!(copied, 300);
        assert_eq!(last.0, 300);
    }

    #[tokio::test]
    async fn copy_rejects_when_source_shrinks_mid_transfer() {
        let payload = vec![0u8; 500];
        let mut reader: &[u8] = payload.as_slice();
        let mut writer: Vec<u8> = Vec::new();

        let err = copy_with_progress(&mut reader, &mut writer, 1000, 64, &mut |_, _| {})
            .await
            .unwrap_err();

        assert!(
            format!("{err:?}").contains("source changed"),
            "实际错误: {err:?}"
        );
    }
}
