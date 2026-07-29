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
    pub async fn new(
        channel: russh::Channel<russh::client::Msg>,
    ) -> Result<Self, ProtocolError> {
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

    /// 上传本地文件到远程
    pub async fn upload(&self, local: &PathBuf, remote: &str) -> Result<u64, ProtocolError> {
        info!(local = %local.display(), remote = %remote, "Uploading file");

        let data = tokio::fs::read(local)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to read local file: {}", e)))?;

        let total = data.len() as u64;

        let mut file = self
            .session
            .create(remote)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to create remote file: {}", e)))?;

        file.write_all(&data)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to write remote file: {}", e)))?;

        // 关闭文件（通过 drop）
        drop(file);

        info!(remote = %remote, bytes = total, "Upload completed");
        Ok(total)
    }

    /// 下载远程文件到本地
    pub async fn download(&self, remote: &str, local: &PathBuf) -> Result<u64, ProtocolError> {
        info!(remote = %remote, local = %local.display(), "Downloading file");

        let mut file = self
            .session
            .open(remote)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to open remote file: {}", e)))?;

        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to read remote file: {}", e)))?;

        let total = data.len() as u64;

        // 确保本地目录存在
        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ProtocolError::ProtocolError(format!("Failed to create local dir: {}", e)))?;
        }

        tokio::fs::write(local, &data)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to write local file: {}", e)))?;

        info!(remote = %remote, bytes = total, "Download completed");
        Ok(total)
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
