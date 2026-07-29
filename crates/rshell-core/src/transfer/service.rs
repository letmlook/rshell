//! 文件传输服务
//!
//! 管理文件传输任务队列，支持上传/下载/暂停/恢复/取消。
//! 实际传输通过 SFTP 客户端执行。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use rshell_api::AppEvent;
use rshell_protocol::ssh::sftp::SftpClient;
use rshell_protocol::ssh::SshClient;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

/// 传输任务状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferTaskState {
    /// 等待中
    Pending,
    /// 传输中
    Transferring,
    /// 已暂停
    Paused,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已取消
    Cancelled,
}

/// 传输方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

/// 传输任务
#[derive(Debug, Clone)]
pub struct TransferTask {
    pub id: Uuid,
    pub session_id: Uuid,
    pub direction: TransferDirection,
    pub local_path: PathBuf,
    pub remote_path: String,
    pub state: TransferTaskState,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub error_message: Option<String>,
    /// 传输开始时间（用于计算速度）
    pub started_at: Option<std::time::Instant>,
    /// 上次更新时间（用于计算速度）
    pub last_update: Option<std::time::Instant>,
    /// 上次更新时的字节数（用于计算速度）
    pub last_bytes: u64,
}

impl TransferTask {
    /// 进度百分比 (0.0 - 1.0)
    pub fn progress(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            self.bytes_transferred as f64 / self.total_bytes as f64
        }
    }
}

/// 文件传输服务
pub struct TransferService {
    /// 传输任务队列
    tasks: Arc<RwLock<HashMap<Uuid, TransferTask>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
    /// 获取 SSH 客户端的函数（由外部注入）
    ssh_client_provider: Arc<RwLock<Option<Arc<dyn Fn(Uuid) -> Result<Arc<tokio::sync::RwLock<SshClient>>, CoreError> + Send + Sync>>>>,
}

impl TransferService {
    /// 创建新的传输服务
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            ssh_client_provider: Arc::new(RwLock::new(None)),
        }
    }

    /// 设置 SSH 客户端提供函数
    pub async fn set_ssh_client_provider(
        &self,
        provider: Arc<dyn Fn(Uuid) -> Result<Arc<tokio::sync::RwLock<SshClient>>, CoreError> + Send + Sync>,
    ) {
        let mut p = self.ssh_client_provider.write().await;
        *p = Some(provider);
    }

    /// 添加上传任务并启动传输
    pub async fn enqueue_upload(
        &self,
        local: PathBuf,
        remote: String,
        session_id: Uuid,
    ) -> Result<Uuid, CoreError> {
        let task_id = Uuid::new_v4();

        let task = TransferTask {
            id: task_id,
            session_id,
            direction: TransferDirection::Upload,
            local_path: local,
            remote_path: remote,
            state: TransferTaskState::Pending,
            bytes_transferred: 0,
            total_bytes: 0,
            error_message: None,
            started_at: None,
            last_update: None,
            last_bytes: 0,
        };

        {
            let mut tasks = self.tasks.write().await;
            tasks.insert(task_id, task);
        }

        info!(task_id = %task_id, "Upload task enqueued");
        self.event_bus.publish(AppEvent::TransferQueueChanged);

        // 启动实际传输
        self.execute_transfer(task_id).await?;

        Ok(task_id)
    }

    /// 添加下载任务并启动传输
    pub async fn enqueue_download(
        &self,
        remote: String,
        local: PathBuf,
        session_id: Uuid,
    ) -> Result<Uuid, CoreError> {
        let task_id = Uuid::new_v4();

        let task = TransferTask {
            id: task_id,
            session_id,
            direction: TransferDirection::Download,
            local_path: local,
            remote_path: remote,
            state: TransferTaskState::Pending,
            bytes_transferred: 0,
            total_bytes: 0,
            error_message: None,
            started_at: None,
            last_update: None,
            last_bytes: 0,
        };

        {
            let mut tasks = self.tasks.write().await;
            tasks.insert(task_id, task);
        }

        info!(task_id = %task_id, "Download task enqueued");
        self.event_bus.publish(AppEvent::TransferQueueChanged);

        // 启动实际传输
        self.execute_transfer(task_id).await?;

        Ok(task_id)
    }

    /// 执行实际的文件传输
    async fn execute_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        let task = {
            let tasks = self.tasks.read().await;
            tasks.get(&task_id).cloned()
        };

        let task = match task {
            Some(t) => t,
            None => return Err(CoreError::NotFound(format!("Task {} not found", task_id))),
        };

        // 更新状态为传输中
        {
            let mut tasks = self.tasks.write().await;
            if let Some(t) = tasks.get_mut(&task_id) {
                t.state = TransferTaskState::Transferring;
                t.started_at = Some(std::time::Instant::now());
                t.last_update = Some(std::time::Instant::now());
                t.last_bytes = 0;
            }
        }

        // 获取 SSH 客户端
        let provider = self.ssh_client_provider.read().await;
        let ssh_client_provider = match provider.as_ref() {
            Some(p) => p.clone(),
            None => {
                let err = "SSH client provider not set".to_string();
                self.mark_failed(task_id, err.clone()).await?;
                return Err(CoreError::Internal(err));
            }
        };

        let ssh_client = match ssh_client_provider(task.session_id) {
            Ok(c) => c,
            Err(e) => {
                let err = format!("Failed to get SSH client: {}", e);
                self.mark_failed(task_id, err.clone()).await?;
                return Err(CoreError::Internal(err));
            }
        };

        // 启动异步传输任务
        let tasks = self.tasks.clone();
        let event_bus = self.event_bus.clone();
        tokio::spawn(async move {
            let result = async {
                let ssh = ssh_client.read().await;
                let channel = ssh
                    .open_sftp_channel()
                    .await
                    .map_err(|e| format!("Failed to open SFTP channel: {}", e))?;

                let sftp = SftpClient::new(channel)
                    .await
                    .map_err(|e| format!("Failed to create SFTP client: {}", e))?;

                match task.direction {
                    TransferDirection::Upload => {
                        let bytes = sftp
                            .upload(&task.local_path, &task.remote_path)
                            .await
                            .map_err(|e| format!("Upload failed: {}", e))?;

                        // 更新进度
                        {
                            let mut tasks = tasks.write().await;
                            if let Some(t) = tasks.get_mut(&task_id) {
                                t.bytes_transferred = bytes;
                                t.total_bytes = bytes;
                            }
                        }

                        event_bus.publish(AppEvent::TransferProgress {
                            task_id,
                            bytes,
                            total: bytes,
                            speed_bps: 0.0,
                        });
                    }
                    TransferDirection::Download => {
                        let bytes = sftp
                            .download(&task.remote_path, &task.local_path)
                            .await
                            .map_err(|e| format!("Download failed: {}", e))?;

                        // 更新进度
                        {
                            let mut tasks = tasks.write().await;
                            if let Some(t) = tasks.get_mut(&task_id) {
                                t.bytes_transferred = bytes;
                                t.total_bytes = bytes;
                            }
                        }

                        event_bus.publish(AppEvent::TransferProgress {
                            task_id,
                            bytes,
                            total: bytes,
                            speed_bps: 0.0,
                        });
                    }
                }

                Ok::<(), String>(())
            }
            .await;

            match result {
                Ok(()) => {
                    let mut tasks = tasks.write().await;
                    if let Some(t) = tasks.get_mut(&task_id) {
                        t.state = TransferTaskState::Completed;
                    }
                    event_bus.publish(AppEvent::TransferCompleted { task_id });
                    event_bus.publish(AppEvent::TransferQueueChanged);
                    info!(task_id = %task_id, "Transfer completed");
                }
                Err(e) => {
                    let mut tasks = tasks.write().await;
                    if let Some(t) = tasks.get_mut(&task_id) {
                        t.state = TransferTaskState::Failed;
                        t.error_message = Some(e.clone());
                    }
                    event_bus.publish(AppEvent::TransferFailed {
                        task_id,
                        error: e.clone(),
                    });
                    event_bus.publish(AppEvent::TransferQueueChanged);
                    warn!(task_id = %task_id, error = %e, "Transfer failed");
                }
            }
        });

        Ok(())
    }

    /// 暂停传输任务
    pub async fn pause_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.state == TransferTaskState::Transferring {
                task.state = TransferTaskState::Paused;
                info!(task_id = %task_id, "Transfer paused");
                self.event_bus.publish(AppEvent::TransferQueueChanged);
            }
        } else {
            warn!(task_id = %task_id, "Transfer task not found");
        }

        Ok(())
    }

    /// 恢复传输任务
    pub async fn resume_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.state == TransferTaskState::Paused {
                task.state = TransferTaskState::Transferring;
                info!(task_id = %task_id, "Transfer resumed");
                self.event_bus.publish(AppEvent::TransferQueueChanged);
            }
        } else {
            warn!(task_id = %task_id, "Transfer task not found");
        }

        Ok(())
    }

    /// 取消传输任务
    pub async fn cancel_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.state != TransferTaskState::Completed {
                task.state = TransferTaskState::Cancelled;
                info!(task_id = %task_id, "Transfer cancelled");
                self.event_bus.publish(AppEvent::TransferQueueChanged);
            }
        } else {
            warn!(task_id = %task_id, "Transfer task not found");
        }

        Ok(())
    }

    /// 更新传输进度
    pub async fn update_progress(
        &self,
        task_id: Uuid,
        bytes_transferred: u64,
        total_bytes: u64,
    ) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            // 计算传输速度
            let speed_bps = if let Some(last_update) = task.last_update {
                let elapsed = last_update.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    let bytes_delta = bytes_transferred.saturating_sub(task.last_bytes);
                    (bytes_delta as f64) / elapsed
                } else {
                    0.0
                }
            } else {
                0.0
            };

            task.bytes_transferred = bytes_transferred;
            task.total_bytes = total_bytes;
            task.last_update = Some(std::time::Instant::now());
            task.last_bytes = bytes_transferred;

            self.event_bus.publish(AppEvent::TransferProgress {
                task_id,
                bytes: bytes_transferred,
                total: total_bytes,
                speed_bps,
            });
        }

        Ok(())
    }

    /// 标记传输完成
    pub async fn mark_completed(&self, task_id: Uuid) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            task.state = TransferTaskState::Completed;
            info!(task_id = %task_id, "Transfer completed");
            self.event_bus.publish(AppEvent::TransferCompleted { task_id });
            self.event_bus.publish(AppEvent::TransferQueueChanged);
        }

        Ok(())
    }

    /// 标记传输失败
    pub async fn mark_failed(&self, task_id: Uuid, error: String) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            task.state = TransferTaskState::Failed;
            task.error_message = Some(error.clone());
            warn!(task_id = %task_id, error = %error, "Transfer failed");
            self.event_bus.publish(AppEvent::TransferFailed { task_id, error });
            self.event_bus.publish(AppEvent::TransferQueueChanged);
        }

        Ok(())
    }

    /// 获取所有任务
    pub async fn list_tasks(&self) -> Vec<TransferTask> {
        let tasks = self.tasks.read().await;
        tasks.values().cloned().collect()
    }

    /// 获取指定任务
    pub async fn get_task(&self, task_id: Uuid) -> Option<TransferTask> {
        let tasks = self.tasks.read().await;
        tasks.get(&task_id).cloned()
    }
}
