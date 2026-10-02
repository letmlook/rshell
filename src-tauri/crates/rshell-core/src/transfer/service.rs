//! 文件传输服务
//!
//! 管理文件传输任务队列，支持上传/下载/暂停/恢复/取消。
//! 实际传输通过 SFTP 客户端执行。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::session::service::validate_remote_mutation_path;
use crate::session::service::SshClientHandle;
use rshell_api::types::{
    TransferDirection as ApiTransferDirection, TransferTaskInfo,
    TransferTaskState as ApiTransferTaskState,
};
use rshell_api::AppEvent;
use rshell_protocol::ssh::sftp::{SftpClient, TransferControl};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};
use tracing::{info, warn};

/// SSH 客户端解析回调（外部注入，避免 TransferService 反向依赖 SessionService）
///
/// 返回 future 是必要的：`SessionService::get_ssh_client` 本身是 async，而
/// `SshClientProvider` 会在多处同步调用，所以必须用 boxed-future 形式。
pub type SshClientProvider = Arc<
    dyn Fn(Uuid) -> Pin<Box<dyn Future<Output = Result<SshClientHandle, CoreError>> + Send>>
        + Send
        + Sync,
>;
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

impl TransferTaskState {
    /// 是否已进入终态（不会再发生状态变化）
    fn is_terminal(self) -> bool {
        matches!(
            self,
            TransferTaskState::Completed | TransferTaskState::Failed | TransferTaskState::Cancelled
        )
    }
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
    /// 当前传输速度（字节/秒）。仅在 Transferring 期间有值，其余状态由前端归零显示。
    pub speed_bps: f64,
    /// 进入终态的时间：终态任务限量清理时据此保留最近 `MAX_FINISHED_TASKS` 条
    pub finished_at: Option<std::time::Instant>,
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

impl From<TransferTask> for TransferTaskInfo {
    fn from(task: TransferTask) -> Self {
        Self {
            id: task.id,
            session_id: task.session_id,
            direction: match task.direction {
                TransferDirection::Upload => ApiTransferDirection::Upload,
                TransferDirection::Download => ApiTransferDirection::Download,
            },
            local_path: task.local_path.to_string_lossy().into_owned(),
            remote_path: task.remote_path,
            state: match task.state {
                TransferTaskState::Pending => ApiTransferTaskState::Pending,
                TransferTaskState::Transferring => ApiTransferTaskState::Transferring,
                TransferTaskState::Paused => ApiTransferTaskState::Paused,
                TransferTaskState::Completed => ApiTransferTaskState::Completed,
                TransferTaskState::Failed => ApiTransferTaskState::Failed,
                TransferTaskState::Cancelled => ApiTransferTaskState::Cancelled,
            },
            bytes_transferred: task.bytes_transferred,
            total_bytes: task.total_bytes,
            speed_bps: task.speed_bps,
            error_message: task.error_message,
        }
    }
}

/// 进度事件最短发布间隔：避免每个分块都触发一次前端全量队列刷新。
const PROGRESS_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// 终态任务（Completed/Failed/Cancelled）的最大保留条数：任务表只保留最近完成的
/// N 条终态任务，防止长期运行进程的内存随历史任务无界增长。
const MAX_FINISHED_TASKS: usize = 50;

/// 进度事件节流：间隔内的帧丢弃，首帧与终帧无条件发出。
struct ProgressThrottle {
    last_sent: Option<std::time::Instant>,
    min_interval: std::time::Duration,
}

impl ProgressThrottle {
    fn new(min_interval: std::time::Duration) -> Self {
        Self {
            last_sent: None,
            min_interval,
        }
    }

    fn should_emit(&mut self, done: u64, total: u64) -> bool {
        let final_frame = total > 0 && done >= total;
        let now = std::time::Instant::now();
        let emit = final_frame
            || match self.last_sent {
                None => true,
                Some(sent) => sent.elapsed() >= self.min_interval,
            };
        if emit {
            self.last_sent = Some(now);
        }
        emit
    }
}

/// 文件传输服务
pub struct TransferService {
    /// 传输任务队列
    tasks: Arc<RwLock<HashMap<Uuid, TransferTask>>>,
    /// 按 task_id 的传输控制通道：pause/resume/cancel 通过它驱动传输循环，
    /// cancel 与 pause 共用同一通道。任务进入终态后移除对应条目。
    control_channels: Arc<RwLock<HashMap<Uuid, watch::Sender<TransferControl>>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
    /// 获取 SSH 客户端的函数（由外部注入）
    ssh_client_provider: Arc<std::sync::RwLock<Option<SshClientProvider>>>,
}

impl TransferService {
    /// 创建新的传输服务
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            control_channels: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            ssh_client_provider: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// 设置 SSH 客户端提供函数
    pub fn set_ssh_client_provider(&self, provider: SshClientProvider) {
        let mut p = self
            .ssh_client_provider
            .write()
            .expect("SSH provider lock poisoned");
        *p = Some(provider);
    }

    /// 添加上传任务并启动传输
    pub async fn enqueue_upload(
        &self,
        local: PathBuf,
        remote: String,
        session_id: Uuid,
    ) -> Result<Uuid, CoreError> {
        validate_remote_mutation_path(&remote)?;
        if !local.is_absolute()
            || !tokio::fs::metadata(&local)
                .await
                .map(|m| m.is_file())
                .unwrap_or(false)
        {
            return Err(CoreError::InvalidState(
                "Upload source must be an existing regular local file".into(),
            ));
        }
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
            speed_bps: 0.0,
            finished_at: None,
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
        validate_remote_mutation_path(&remote)?;
        if !local.is_absolute() || local.file_name().is_none() || local.exists() {
            return Err(CoreError::InvalidState(
                "Download target must be a new absolute local file".into(),
            ));
        }
        if !local.parent().is_some_and(|p| p.is_dir()) {
            return Err(CoreError::InvalidState(
                "Download target directory does not exist".into(),
            ));
        }
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
            speed_bps: 0.0,
            finished_at: None,
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
                if t.state == TransferTaskState::Cancelled {
                    // 启动前已被取消：不启动传输，保持 Cancelled 终态
                    info!(task_id = %task_id, "Transfer cancelled before start; not executing");
                    return Ok(());
                }
                t.state = TransferTaskState::Transferring;
                t.started_at = Some(std::time::Instant::now());
                t.last_update = Some(std::time::Instant::now());
                t.last_bytes = 0;
            }
        }

        // 获取 SSH 客户端
        let provider = self
            .ssh_client_provider
            .read()
            .expect("SSH provider lock poisoned")
            .clone();
        let ssh_client_provider = match provider {
            Some(p) => p,
            None => {
                let err = "SSH client provider not set".to_string();
                self.mark_failed(task_id, err.clone()).await?;
                return Err(CoreError::Internal(err));
            }
        };

        let ssh_client = match ssh_client_provider(task.session_id).await {
            Ok(c) => c,
            Err(e) => {
                let err = format!("Failed to get SSH client: {}", e);
                self.mark_failed(task_id, err.clone()).await?;
                return Err(CoreError::Internal(err));
            }
        };

        // 建立 pause/resume/cancel 共用的控制通道，传输循环在每个分块前检查
        let (control_tx, mut control_rx) = watch::channel(TransferControl::Run);
        {
            let mut controls = self.control_channels.write().await;
            controls.insert(task_id, control_tx);
        }

        // 启动异步传输任务
        let tasks = self.tasks.clone();
        let control_channels = self.control_channels.clone();
        let event_bus = self.event_bus.clone();
        tokio::spawn(async move {
            // SFTP 拷贝循环里只能同步回调（不能在回调里 await 写锁），
            // 所以回调只做节流后投递，由独立 forwarder 任务落表并广播。
            let (progress_tx, mut progress_rx) =
                tokio::sync::mpsc::unbounded_channel::<(u64, u64)>();
            let forward_tasks = tasks.clone();
            let forward_bus = event_bus.clone();
            let forwarder = tokio::spawn(async move {
                while let Some((bytes, total)) = progress_rx.recv().await {
                    TransferService::apply_progress(
                        &forward_tasks,
                        &forward_bus,
                        task_id,
                        bytes,
                        total,
                    )
                    .await;
                }
            });

            let mut throttle = ProgressThrottle::new(PROGRESS_MIN_INTERVAL);
            let mut progress = move |done: u64, total: u64| {
                if throttle.should_emit(done, total) {
                    let _ = progress_tx.send((done, total));
                }
            };

            let result = async {
                // 只在打开 SFTP 子通道时持有客户端读锁；SFTP 流独立于发送客户端，
                // 拷贝循环（含暂停挂起期间）不得阻塞会话断开等需要写锁的操作。
                let sftp = {
                    let ssh = ssh_client.read().await;
                    let channel = ssh
                        .open_sftp_channel()
                        .await
                        .map_err(|e| format!("Failed to open SFTP channel: {}", e))?;

                    SftpClient::new(channel)
                        .await
                        .map_err(|e| format!("Failed to create SFTP client: {}", e))?
                };

                match task.direction {
                    TransferDirection::Upload => {
                        sftp.upload(
                            &task.local_path,
                            &task.remote_path,
                            &mut control_rx,
                            &mut progress,
                        )
                        .await
                        .map_err(|e| format!("Upload failed: {}", e))?;
                    }
                    TransferDirection::Download => {
                        sftp.download(
                            &task.remote_path,
                            &task.local_path,
                            &mut control_rx,
                            &mut progress,
                        )
                        .await
                        .map_err(|e| format!("Download failed: {}", e))?;
                    }
                }

                Ok::<(), String>(())
            }
            .await;

            // 关闭发送端 → forwarder 排空最后一批进度后自行退出
            drop(progress);
            let _ = forwarder.await;

            // 清理控制通道：此后对该任务的 pause/resume/cancel 只改任务表状态
            {
                let mut controls = control_channels.write().await;
                controls.remove(&task_id);
            }

            // 终态写回：已被取消的任务保持 Cancelled，不广播 TransferCompleted/TransferFailed
            Self::finalize_transfer(&tasks, &event_bus, task_id, result).await;
        });

        Ok(())
    }

    /// 传输循环结束后的终态写回
    ///
    /// 取消命令可能先于循环结束到达（任务表已被标记 Cancelled）：此时保持
    /// Cancelled 终态，只广播队列刷新，不发 TransferCompleted/TransferFailed，
    /// 避免「已取消」被无条件覆盖为完成。
    async fn finalize_transfer(
        tasks: &Arc<RwLock<HashMap<Uuid, TransferTask>>>,
        event_bus: &Arc<EventBus>,
        task_id: Uuid,
        result: Result<(), String>,
    ) {
        match result {
            Ok(()) => {
                let mut tasks = tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    if t.state == TransferTaskState::Cancelled {
                        info!(task_id = %task_id, "Transfer loop finished but task already cancelled; keeping cancelled state");
                        event_bus.publish(AppEvent::TransferQueueChanged);
                    } else {
                        t.state = TransferTaskState::Completed;
                        t.finished_at = Some(std::time::Instant::now());
                        event_bus.publish(AppEvent::TransferCompleted { task_id });
                        event_bus.publish(AppEvent::TransferQueueChanged);
                        info!(task_id = %task_id, "Transfer completed");
                    }
                }
                Self::prune_finished_tasks(&mut tasks);
            }
            Err(e) => {
                let mut tasks = tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    if t.state == TransferTaskState::Cancelled {
                        // 循环因取消而中止：这不是失败，保持 Cancelled
                        info!(task_id = %task_id, "Transfer aborted by cancel; keeping cancelled state");
                        event_bus.publish(AppEvent::TransferQueueChanged);
                    } else {
                        t.state = TransferTaskState::Failed;
                        t.error_message = Some(e.clone());
                        t.finished_at = Some(std::time::Instant::now());
                        event_bus.publish(AppEvent::TransferFailed {
                            task_id,
                            error: e.clone(),
                        });
                        event_bus.publish(AppEvent::TransferQueueChanged);
                        warn!(task_id = %task_id, error = %e, "Transfer failed");
                    }
                }
                Self::prune_finished_tasks(&mut tasks);
            }
        }
    }

    /// 终态任务限量清理：只保留最近 `MAX_FINISHED_TASKS` 条终态任务，
    /// 防止长期运行进程中任务表随历史任务无界增长。
    /// 活跃任务（Pending/Transferring/Paused）永远不会被清理。
    fn prune_finished_tasks(tasks: &mut HashMap<Uuid, TransferTask>) {
        let mut finished: Vec<(Option<std::time::Instant>, Uuid)> = tasks
            .iter()
            .filter(|(_, task)| task.state.is_terminal())
            .map(|(id, task)| (task.finished_at, *id))
            .collect();
        if finished.len() <= MAX_FINISHED_TASKS {
            return;
        }
        // 从最早的开始移除；finished_at 为 None 视作最早，时间并列按 task_id 保证确定性
        finished.sort_by_key(|(finished_at, id)| (*finished_at, *id));
        let excess = finished.len() - MAX_FINISHED_TASKS;
        for (_, id) in finished.into_iter().take(excess) {
            tasks.remove(&id);
        }
    }

    /// 暂停传输任务
    ///
    /// 置 Paused 并通过控制通道通知传输循环在下一分块前挂起；
    /// 字节随即停止增长，恢复后从已传字节继续。
    pub async fn pause_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        // 先克隆发送端再锁任务表，保持 control_channels → tasks 的加锁顺序
        let control = self.control_channels.read().await.get(&task_id).cloned();
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.state == TransferTaskState::Transferring {
                task.state = TransferTaskState::Paused;
                // 传输循环已自行结束时发送失败（无接收端），按终态写回逻辑收尾
                if let Some(control) = control {
                    let _ = control.send(TransferControl::Pause);
                }
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
        let control = self.control_channels.read().await.get(&task_id).cloned();
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.state == TransferTaskState::Paused {
                task.state = TransferTaskState::Transferring;
                // 唤醒挂起中的传输循环；发送失败说明循环已结束，由终态写回收尾
                if let Some(control) = control {
                    let _ = control.send(TransferControl::Run);
                }
                info!(task_id = %task_id, "Transfer resumed");
                self.event_bus.publish(AppEvent::TransferQueueChanged);
            }
        } else {
            warn!(task_id = %task_id, "Transfer task not found");
        }

        Ok(())
    }

    /// 取消传输任务
    ///
    /// 置 Cancelled 并通过控制通道通知传输循环在下一分块前中止；
    /// 循环结束后的终态写回会保持 Cancelled，不广播 TransferCompleted。
    pub async fn cancel_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        let control = self.control_channels.read().await.get(&task_id).cloned();
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.state != TransferTaskState::Completed {
                task.state = TransferTaskState::Cancelled;
                task.finished_at = Some(std::time::Instant::now());
                if let Some(control) = control {
                    let _ = control.send(TransferControl::Cancel);
                }
                info!(task_id = %task_id, "Transfer cancelled");
                self.event_bus.publish(AppEvent::TransferQueueChanged);
            }
        } else {
            warn!(task_id = %task_id, "Transfer task not found");
        }
        Self::prune_finished_tasks(&mut tasks);

        Ok(())
    }

    /// 更新传输进度
    ///
    /// 抽成关联函数 `apply_progress` 是为了让 execute_transfer 里的
    /// forwarder 任务只持有 `tasks` / `event_bus` 的克隆即可复用同一逻辑。
    pub async fn update_progress(
        &self,
        task_id: Uuid,
        bytes_transferred: u64,
        total_bytes: u64,
    ) -> Result<(), CoreError> {
        Self::apply_progress(
            &self.tasks,
            &self.event_bus,
            task_id,
            bytes_transferred,
            total_bytes,
        )
        .await;
        Ok(())
    }

    /// 更新进度并广播：写任务表（含速度计算）+ 发布 `TransferProgress`
    async fn apply_progress(
        tasks: &Arc<RwLock<HashMap<Uuid, TransferTask>>>,
        event_bus: &Arc<EventBus>,
        task_id: Uuid,
        bytes_transferred: u64,
        total_bytes: u64,
    ) {
        let mut tasks = tasks.write().await;
        let Some(task) = tasks.get_mut(&task_id) else {
            return;
        };

        // 速度 = 自上次更新以来的字节增量 / 间隔（用更新前的旧值计算）
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
        task.speed_bps = speed_bps;

        event_bus.publish(AppEvent::TransferProgress {
            task_id,
            bytes: bytes_transferred,
            total: total_bytes,
            speed_bps,
        });
    }

    /// 标记传输完成
    pub async fn mark_completed(&self, task_id: Uuid) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            task.state = TransferTaskState::Completed;
            task.finished_at = Some(std::time::Instant::now());
            info!(task_id = %task_id, "Transfer completed");
            self.event_bus
                .publish(AppEvent::TransferCompleted { task_id });
            self.event_bus.publish(AppEvent::TransferQueueChanged);
        }
        Self::prune_finished_tasks(&mut tasks);

        Ok(())
    }

    /// 标记传输失败
    pub async fn mark_failed(&self, task_id: Uuid, error: String) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            task.state = TransferTaskState::Failed;
            task.error_message = Some(error.clone());
            task.finished_at = Some(std::time::Instant::now());
            warn!(task_id = %task_id, error = %error, "Transfer failed");
            self.event_bus
                .publish(AppEvent::TransferFailed { task_id, error });
            self.event_bus.publish(AppEvent::TransferQueueChanged);
        }
        Self::prune_finished_tasks(&mut tasks);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_service() -> TransferService {
        TransferService::new(Arc::new(crate::event_bus::EventBus::new()))
    }

    #[tokio::test]
    async fn enqueue_upload_rejects_missing_file_before_queueing() {
        let service = make_service();
        let err = service
            .enqueue_upload(
                PathBuf::from("/definitely/missing/rshell-file"),
                "/remote/file".into(),
                Uuid::new_v4(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidState(_)));
        assert!(service.list_tasks().await.is_empty());
    }

    #[tokio::test]
    async fn enqueue_download_rejects_root_and_existing_target() {
        let service = make_service();
        let folder = tempfile::tempdir().unwrap();
        let target = folder.path().join("file");
        assert!(matches!(
            service
                .enqueue_download("/".into(), target.clone(), Uuid::new_v4())
                .await,
            Err(CoreError::InvalidState(_))
        ));
        std::fs::write(&target, b"keep").unwrap();
        assert!(matches!(
            service
                .enqueue_download("/remote/file".into(), target.clone(), Uuid::new_v4())
                .await,
            Err(CoreError::InvalidState(_))
        ));
        assert_eq!(std::fs::read(target).unwrap(), b"keep");
        assert!(service.list_tasks().await.is_empty());
    }

    fn make_task(id: Uuid, state: TransferTaskState) -> TransferTask {
        TransferTask {
            id,
            session_id: Uuid::new_v4(),
            direction: TransferDirection::Upload,
            local_path: PathBuf::from("/tmp/test"),
            remote_path: "/remote/test".to_string(),
            state,
            bytes_transferred: 0,
            total_bytes: 100,
            error_message: None,
            started_at: None,
            last_update: None,
            last_bytes: 0,
            speed_bps: 0.0,
            finished_at: None,
        }
    }

    #[tokio::test]
    async fn test_pause_only_affects_transferring_task() {
        let svc = make_service();
        let id = Uuid::new_v4();

        // 直接塞一个 Transferring 任务
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Transferring));
        }

        svc.pause_transfer(id).await.unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Paused);
    }

    #[tokio::test]
    async fn test_pause_pending_task_is_noop() {
        let svc = make_service();
        let id = Uuid::new_v4();

        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Pending));
        }

        svc.pause_transfer(id).await.unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Pending); // 未变化
    }

    #[tokio::test]
    async fn test_resume_only_affects_paused_task() {
        let svc = make_service();
        let id = Uuid::new_v4();

        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Paused));
        }

        svc.resume_transfer(id).await.unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Transferring);
    }

    #[tokio::test]
    async fn test_resume_completed_is_noop() {
        let svc = make_service();
        let id = Uuid::new_v4();

        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Completed));
        }

        svc.resume_transfer(id).await.unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Completed); // 未变化
    }

    #[tokio::test]
    async fn test_cancel_not_completed_task() {
        let svc = make_service();
        let id = Uuid::new_v4();

        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Transferring));
        }

        svc.cancel_transfer(id).await.unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Cancelled);
    }

    #[tokio::test]
    async fn test_cancel_completed_is_noop() {
        let svc = make_service();
        let id = Uuid::new_v4();

        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Completed));
        }

        svc.cancel_transfer(id).await.unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Completed); // 未变化
    }

    /// 在服务的控制通道上注册发送端，返回可在测试中观察的接收端
    async fn register_control(
        svc: &TransferService,
        task_id: Uuid,
    ) -> watch::Receiver<TransferControl> {
        let (tx, rx) = watch::channel(TransferControl::Run);
        svc.control_channels.write().await.insert(task_id, tx);
        rx
    }

    #[tokio::test]
    async fn pause_transfer_signals_pause_on_control_channel() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Transferring));
        }
        let rx = register_control(&svc, id).await;

        svc.pause_transfer(id).await.unwrap();

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Paused);
        assert_eq!(*rx.borrow(), TransferControl::Pause, "必须通知传输循环挂起");
    }

    #[tokio::test]
    async fn resume_transfer_signals_run_on_control_channel() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Paused));
        }
        let rx = register_control(&svc, id).await;

        svc.resume_transfer(id).await.unwrap();

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Transferring);
        assert_eq!(*rx.borrow(), TransferControl::Run, "必须唤醒传输循环");
    }

    #[tokio::test]
    async fn cancel_transfer_signals_cancel_on_control_channel() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Transferring));
        }
        let rx = register_control(&svc, id).await;

        svc.cancel_transfer(id).await.unwrap();

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Cancelled);
        assert_eq!(
            *rx.borrow(),
            TransferControl::Cancel,
            "必须通知传输循环中止"
        );
    }

    fn collect_events(svc: &TransferService) -> Arc<std::sync::Mutex<Vec<AppEvent>>> {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        svc.event_bus
            .subscribe(move |event| sink.lock().unwrap().push(event.clone()));
        seen
    }

    #[tokio::test]
    async fn finalize_keeps_cancelled_state_and_skips_terminal_events() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Cancelled));
        }
        let seen = collect_events(&svc);

        // 取消先于循环结束到达：Ok 与 Err 结果都必须保持 Cancelled
        TransferService::finalize_transfer(&svc.tasks, &svc.event_bus, id, Ok(())).await;
        assert_eq!(
            svc.get_task(id).await.unwrap().state,
            TransferTaskState::Cancelled
        );

        TransferService::finalize_transfer(&svc.tasks, &svc.event_bus, id, Err("aborted".into()))
            .await;
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Cancelled);
        assert!(t.error_message.is_none(), "取消不是失败，不得写入错误信息");

        let events = seen.lock().unwrap();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AppEvent::TransferCompleted { .. })),
            "取消后不得广播 TransferCompleted"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AppEvent::TransferFailed { .. })),
            "取消后不得广播 TransferFailed"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AppEvent::TransferQueueChanged)),
            "终态写回后应广播队列刷新"
        );
    }

    #[tokio::test]
    async fn finalize_marks_completed_and_failed_when_not_cancelled() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Transferring));
        }
        let seen = collect_events(&svc);

        TransferService::finalize_transfer(&svc.tasks, &svc.event_bus, id, Ok(())).await;
        assert_eq!(
            svc.get_task(id).await.unwrap().state,
            TransferTaskState::Completed
        );
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, AppEvent::TransferCompleted { task_id } if *task_id == id)));

        // 失败路径：重置为传输中后以 Err 收尾
        {
            let mut tasks = svc.tasks.write().await;
            tasks.get_mut(&id).unwrap().state = TransferTaskState::Transferring;
        }
        TransferService::finalize_transfer(&svc.tasks, &svc.event_bus, id, Err("disk full".into()))
            .await;
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Failed);
        assert_eq!(t.error_message.as_deref(), Some("disk full"));
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, AppEvent::TransferFailed { task_id, .. } if *task_id == id)));
    }

    #[tokio::test]
    async fn execute_transfer_skips_start_when_cancelled_before_start() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Pending));
        }
        let seen = collect_events(&svc);

        // 入队与启动之间的取消竞态：任务已被标记 Cancelled
        svc.cancel_transfer(id).await.unwrap();
        assert_eq!(
            svc.get_task(id).await.unwrap().state,
            TransferTaskState::Cancelled
        );

        svc.execute_transfer(id).await.unwrap();

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(
            t.state,
            TransferTaskState::Cancelled,
            "启动前取消的任务不得被改回传输中/失败"
        );
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, AppEvent::TransferFailed { .. })),
            "未启动的取消不得广播 TransferFailed"
        );
    }

    #[tokio::test]
    async fn test_mark_completed() {
        let svc = make_service();
        let id = Uuid::new_v4();

        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Transferring));
        }

        svc.mark_completed(id).await.unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Completed);
    }

    #[tokio::test]
    async fn test_mark_failed() {
        let svc = make_service();
        let id = Uuid::new_v4();

        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Transferring));
        }

        svc.mark_failed(id, "connection reset".to_string())
            .await
            .unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Failed);
        assert_eq!(t.error_message.as_deref(), Some("connection reset"));
    }

    #[tokio::test]
    async fn test_list_tasks() {
        let svc = make_service();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id1, make_task(id1, TransferTaskState::Transferring));
            tasks.insert(id2, make_task(id2, TransferTaskState::Paused));
        }

        let all = svc.list_tasks().await;
        assert_eq!(all.len(), 2);
        assert!(all
            .iter()
            .any(|t| t.state == TransferTaskState::Transferring));
        assert!(all.iter().any(|t| t.state == TransferTaskState::Paused));
    }

    #[tokio::test]
    async fn test_get_task_not_found() {
        let svc = make_service();
        assert!(svc.get_task(Uuid::new_v4()).await.is_none());
    }

    #[test]
    fn test_transfer_task_progress() {
        let task = TransferTask {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            direction: TransferDirection::Upload,
            local_path: PathBuf::from("/tmp/test"),
            remote_path: "/remote/test".to_string(),
            state: TransferTaskState::Transferring,
            bytes_transferred: 50,
            total_bytes: 100,
            error_message: None,
            started_at: None,
            last_update: None,
            last_bytes: 0,
            speed_bps: 0.0,
            finished_at: None,
        };
        assert!((task.progress() - 0.5).abs() < f64::EPSILON);

        // total_bytes == 0 不会 panic
        let zero = TransferTask {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            direction: TransferDirection::Download,
            local_path: PathBuf::from("/tmp/test2"),
            remote_path: "/remote/test2".to_string(),
            state: TransferTaskState::Transferring,
            bytes_transferred: 0,
            total_bytes: 0,
            error_message: None,
            started_at: None,
            last_update: None,
            last_bytes: 0,
            speed_bps: 0.0,
            finished_at: None,
        };
        assert_eq!(zero.progress(), 0.0);
    }

    #[test]
    fn progress_throttle_emits_first_immediately_and_final_frame_always() {
        let mut t = ProgressThrottle::new(std::time::Duration::from_secs(60));
        assert!(t.should_emit(0, 1000), "首帧必须立即发出");
        assert!(!t.should_emit(128, 1000), "间隔内的中间帧要被节流");
        assert!(!t.should_emit(256, 1000));
        assert!(t.should_emit(1000, 1000), "终帧必须无条件发出");

        let mut fast = ProgressThrottle::new(std::time::Duration::ZERO);
        assert!(fast.should_emit(1, 1000));
        assert!(fast.should_emit(2, 1000));
    }

    #[tokio::test]
    async fn apply_progress_updates_task_and_publishes_nonzero_speed() {
        let event_bus = Arc::new(EventBus::new());
        let svc = TransferService::new(event_bus.clone());
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            let mut task = make_task(id, TransferTaskState::Transferring);
            // 500ms 前进过 0 字节 → 500 字节 / 0.5s = 1000 B/s
            task.last_update =
                Some(std::time::Instant::now() - std::time::Duration::from_millis(500));
            task.last_bytes = 0;
            tasks.insert(id, task);
        }

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        event_bus.subscribe(move |event| {
            if let AppEvent::TransferProgress {
                bytes,
                total,
                speed_bps,
                ..
            } = event
            {
                sink.lock().unwrap().push((*bytes, *total, *speed_bps));
            }
        });

        TransferService::apply_progress(&svc.tasks, &svc.event_bus, id, 500, 1000).await;

        {
            let tasks = svc.tasks.read().await;
            let task = tasks.get(&id).unwrap();
            assert_eq!(task.bytes_transferred, 500);
            assert_eq!(task.total_bytes, 1000);
            assert!(task.speed_bps > 0.0, "speed_bps 应为正数");
        }

        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].0, events[0].1), (500, 1000));
        assert!(events[0].2 > 0.0, "广播出去的速度也要是正数");
    }

    /// 把任务直接插入任务表（绕过入队校验，便于构造测试场景）
    async fn insert_task(svc: &TransferService, task: TransferTask) {
        svc.tasks.write().await.insert(task.id, task);
    }

    fn task_finished_long_ago(id: Uuid, state: TransferTaskState) -> TransferTask {
        let mut task = make_task(id, state);
        task.finished_at =
            std::time::Instant::now().checked_sub(std::time::Duration::from_secs(3600));
        task
    }

    /// 验收标准：完成 MAX_FINISHED_TASKS + 1 个任务后，任务表长度必须 ≤ MAX_FINISHED_TASKS
    #[tokio::test]
    async fn finished_tasks_are_pruned_to_keep_recent_max() {
        let svc = make_service();
        let mut old_ids = Vec::new();

        for _ in 0..MAX_FINISHED_TASKS {
            let id = Uuid::new_v4();
            insert_task(
                &svc,
                task_finished_long_ago(id, TransferTaskState::Completed),
            )
            .await;
            old_ids.push(id);
        }

        // 第 MAX+1 个任务走真实终态转换，触发限量清理
        let newest = Uuid::new_v4();
        insert_task(&svc, make_task(newest, TransferTaskState::Transferring)).await;
        svc.mark_completed(newest).await.unwrap();

        let all = svc.list_tasks().await;
        assert!(
            all.len() <= MAX_FINISHED_TASKS,
            "完成 MAX+1 个任务后任务表长度必须 ≤ MAX，实际 {}",
            all.len()
        );
        assert!(all.iter().any(|t| t.id == newest), "最近完成的任务必须保留");
        let remaining_old = all.iter().filter(|t| old_ids.contains(&t.id)).count();
        assert_eq!(
            remaining_old,
            MAX_FINISHED_TASKS - 1,
            "被清理的只能是最旧的终态任务"
        );
    }

    /// 限量清理只针对终态任务，活跃任务（Pending/Transferring/Paused）永不清理
    #[tokio::test]
    async fn pruning_never_removes_active_tasks() {
        let svc = make_service();
        let mut active_ids = Vec::new();
        for state in [
            TransferTaskState::Pending,
            TransferTaskState::Transferring,
            TransferTaskState::Paused,
        ] {
            let id = Uuid::new_v4();
            insert_task(&svc, make_task(id, state)).await;
            active_ids.push(id);
        }

        for _ in 0..MAX_FINISHED_TASKS {
            let id = Uuid::new_v4();
            insert_task(&svc, task_finished_long_ago(id, TransferTaskState::Failed)).await;
        }
        // 超限后触发一次真实终态转换
        let id = Uuid::new_v4();
        insert_task(&svc, make_task(id, TransferTaskState::Transferring)).await;
        svc.mark_failed(id, "boom".to_string()).await.unwrap();

        let all = svc.list_tasks().await;
        for active in &active_ids {
            assert!(
                all.iter().any(|t| t.id == *active),
                "活跃任务不得被限量清理"
            );
        }
        let finished_count = all.iter().filter(|t| t.state.is_terminal()).count();
        assert!(
            finished_count <= MAX_FINISHED_TASKS,
            "终态任务数量必须 ≤ MAX，实际 {}",
            finished_count
        );
    }

    /// 取消（含启动前取消）也是终态转换，同样记录终态时间并参与限量清理
    #[tokio::test]
    async fn cancelled_tasks_record_finished_at() {
        let svc = make_service();
        let id = Uuid::new_v4();
        insert_task(&svc, make_task(id, TransferTaskState::Transferring)).await;

        svc.cancel_transfer(id).await.unwrap();

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Cancelled);
        assert!(t.finished_at.is_some(), "取消后必须记录终态时间");
    }
}
