//! 文件传输服务
//!
//! 管理文件传输任务队列，支持上传/下载/暂停/恢复/取消。
//! 实际传输通过 SFTP 客户端执行。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::session::service::validate_remote_mutation_path;
use crate::session::service::SshClientHandle;
use crate::transfer::staged::{
    run_staged_download, run_staged_upload, staging_local_temp_path, staging_temp_path_str,
    DownloadRequest, FailedStage, StagedOutcome, UploadRequest, DECISION_CANCELLED,
    DECISION_PENDING,
};
use rshell_api::types::{
    ConflictPolicy, TransferDirection as ApiTransferDirection, TransferTaskInfo,
    TransferTaskState as ApiTransferTaskState,
};
use rshell_api::AppEvent;
use rshell_protocol::ssh::sftp::{CommitStrategy, SftpClient, TransferControl, TransferSink};
use rshell_protocol::transfer::LocalSink;
use rshell_protocol::ProtocolError;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::AtomicU8;
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
    /// 目标已存在时的策略。上传在 SFTP `create` 之前强制执行（防竞态覆盖），
    /// 下载在入队时用本地文件系统检查（无需额外往返）。
    pub conflict: ConflictPolicy,
    // ── R2-T2 字段 ──
    /// 该任务拥有的 staging temp 路径（远端侧 / 本地侧）。
    /// 成功路径上提交 rename 后该字段**仍保留**，用于「清理失败 → 残留」的展示。
    /// 在任务进入 `Completed` 终态时由 staged lifecycle 标记完成。
    pub temp_path: Option<String>,
    /// 终态下的清理结果 —— 失败时 residue 路径与原因。
    pub cleanup_status: Option<CleanupStatus>,
    /// 提交结果：仅成功路径上为 `Some(Committed(_))`，
    /// 失败/取消路径为 None。
    pub commit_status: Option<CommitStatus>,
}

/// R2-T2：终态下的清理结果。
///
/// `Cleaned` 表示 temp 已被成功移除（成功路径）或清理失败路径上的清理成功；
/// `Residue { path, reason }` 表示清理失败 —— temp 仍存在，路径与原因如实告诉用户。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupStatus {
    /// temp 已被成功移除
    Cleaned,
    /// temp 仍存在 —— 用户可见的「残留」
    Residue { path: String, reason: String },
}

impl CleanupStatus {
    /// 终态展示 —— 直接给前端 `TransferTaskInfo` 的清理状态字段用
    pub fn label(&self) -> &'static str {
        match self {
            CleanupStatus::Cleaned => "cleaned",
            CleanupStatus::Residue { .. } => "residue",
        }
    }
}

/// R2-T2：提交结果。
///
/// 仅在 `Completed` 终态上为 `Some(Committed(strategy))`；其余终态为 None
/// （失败 / 取消路径上提交并未发生）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStatus {
    Committed(CommitStrategy),
}

impl CommitStatus {
    pub fn strategy(&self) -> CommitStrategy {
        match self {
            CommitStatus::Committed(s) => *s,
        }
    }
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

    /// R2-T2：算出 staging temp 路径 —— `execute_transfer` 启动前需要它写
    /// 回 `temp_path` 字段，以便取消 / cleanup 失败时知道路径。启动窗口
    /// 内 `temp_path` 已是 Some 直接返回；否则按 task_id + 方向重算。
    pub fn temp_path_for(&self, task_id: Uuid) -> Option<String> {
        if let Some(p) = &self.temp_path {
            return Some(p.clone());
        }
        let id_str = task_id.to_string();
        match self.direction {
            TransferDirection::Upload => Some(staging_temp_path_str(&self.remote_path, &id_str)),
            TransferDirection::Download => Some(
                staging_local_temp_path(&self.local_path, &id_str)
                    .to_string_lossy()
                    .into_owned(),
            ),
        }
    }
}

impl FailedStage {
    /// R2-T2：失败阶段的可读标签 —— 写进 `error_message` 前缀供前端展示。
    pub fn label(&self) -> &'static str {
        match self {
            FailedStage::ExclusiveOpen => "exclusive_open",
            FailedStage::Copy => "copy",
            FailedStage::LateConflict => "late_conflict",
            FailedStage::Commit => "commit",
            FailedStage::CancelledBeforeCommit => "cancelled_before_commit",
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
            temp_path: task.temp_path,
            cleanup_status: task.cleanup_status.as_ref().map(|s| s.label().to_string()),
            commit_strategy: task
                .commit_status
                .as_ref()
                .map(|c| commit_strategy_label(c.strategy()).to_string()),
        }
    }
}

/// R2-T2：把 `CommitStrategy` 映射成给前端的稳定字符串。
///
/// 仅作前端展示用 —— 实际含义在 `CommitStrategy` 的注释里。这里
/// 是为了让 `TransferTaskInfo` 的 JSON 字段保持稳定的字符串集合。
pub fn commit_strategy_label(strategy: CommitStrategy) -> &'static str {
    match strategy {
        CommitStrategy::PosixRename => "posix_rename",
        CommitStrategy::StandardRename => "standard_rename",
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

/// 启动前的控制收口：暂停挂起、取消中止（与 sftp.rs 的 `wait_for_run` 同语义，
/// 但作用于「打开 SFTP 通道之前」）——启动窗口内到达的暂停不会打开任何远端通道。
async fn wait_for_run_before_start(
    control: &mut watch::Receiver<TransferControl>,
) -> Result<(), String> {
    loop {
        match *control.borrow_and_update() {
            TransferControl::Cancel => return Err("transfer cancelled before start".to_string()),
            TransferControl::Run => return Ok(()),
            TransferControl::Pause => {}
        }
        control
            .changed()
            .await
            .map_err(|_| "transfer cancelled before start".to_string())?;
    }
}

/// R2-17：传输任务兜底清理守卫。
///
/// 任务体（tokio::spawn 的 async 块）在正常路径末尾 disarm；若中途 unwind
///（panic），Drop 在任务自身的运行时上下文内再 spawn 一个后台清理：移除
/// 控制通道条目并按失败写回终态——否则任务表永久停在非终态、control_channels
/// 泄漏条目，后续 pause/resume/cancel 会对幽灵任务操作。
struct TransferCleanupGuard {
    task_id: Uuid,
    tasks: Arc<RwLock<HashMap<Uuid, TransferTask>>>,
    control_channels: Arc<RwLock<HashMap<Uuid, watch::Sender<TransferControl>>>>,
    event_bus: Arc<EventBus>,
    armed: bool,
}

impl TransferCleanupGuard {
    /// 正常清理路径（控制通道移除 + 终态写回）完成后调用：Drop 不再兜底
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TransferCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Drop 运行在任务自己的运行时上下文里（同步上下文不能 await），
        // 把兜底清理作为独立任务 spawn 出去。
        let task_id = self.task_id;
        let tasks = self.tasks.clone();
        let control_channels = self.control_channels.clone();
        let event_bus = self.event_bus.clone();
        tokio::spawn(async move {
            control_channels.write().await.remove(&task_id);
            TransferService::finalize_transfer(
                &tasks,
                &event_bus,
                task_id,
                Err("transfer task panicked before finishing".to_string()),
            )
            .await;
        });
    }
}

/// 冲突标记前缀：写入 `error_message`，前端据此在队列行上提供「覆盖/重命名」。
/// 固定字符串而非整句文案解析——文案会随语言/版本变化。
pub const TRANSFER_CONFLICT_PREFIX: &str = "target_exists: ";

/// 传输错误里若含冲突标记则取出目标路径
fn conflict_target(error: &str) -> Option<String> {
    error
        .strip_prefix(TRANSFER_CONFLICT_PREFIX)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// 把协议层错误转成可展示的文案；冲突统一加上标记前缀。
///
/// `ProtocolError::TransferConflict(path)` 的 Display 形如
/// `Transfer target already exists: /path`，这里剥出裸路径供前端定位。
fn describe_transfer_error(e: &str) -> String {
    match e.strip_prefix("Transfer target already exists: ") {
        Some(path) if !path.is_empty() => format!("{TRANSFER_CONFLICT_PREFIX}{path}"),
        _ => e.to_string(),
    }
}

/// R3-04：文件名必须是**单个路径分量**——不含分隔符、不为 `.`/`..`、非空非空白。
///
/// 远端版（[`sibling_path`]）与本地版（[`resolve_local_target`]）共用此判定。
/// 原因：`Path::with_file_name` 只替换最后一个分量而**不做任何净化**，含 `/`、
/// `\`、`..` 或驱动器前缀的名字会让结果逃出目标目录：
///
/// ```text
/// C:\Users\me\Downloads\report.txt + "..\..\Startup\evil.exe"
///   => C:\Users\me\Downloads\..\..\Startup\evil.exe   （解析后已逃出）
/// /home/user/downloads/report.txt  + "C:evil.txt"
///   => C:evil.txt                                     （整条路径被替换）
/// ```
///
/// 漏掉这一层，「重命名」就成了任意路径写入的旁路。
fn is_unsafe_component(name: &str) -> bool {
    name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || name.trim().is_empty()
}

/// 由「所在目录 + 新文件名」拼出目标路径。
///
/// 拒绝含分隔符、`.`/`..`、空白的名字——否则「重命名」会成为任意路径写入的
/// 旁路，绕过 `validate_remote_mutation_path` 的越界检查。
pub fn sibling_path(base: &str, name: &str) -> Result<String, CoreError> {
    if is_unsafe_component(name) {
        return Err(CoreError::InvalidState(format!("Unsafe file name: {name}")));
    }
    let dir = match base.rfind('/') {
        // 命中 0 表示 "/name"，父目录就是根
        Some(0) => "/".to_string(),
        Some(i) => base[..i].to_string(),
        None => {
            return Err(CoreError::InvalidState(format!(
                "Base path has no parent: {base}"
            )))
        }
    };
    if dir == "/" {
        Ok(format!("/{name}"))
    } else {
        Ok(format!("{dir}/{name}"))
    }
}

/// 本地下载目标解析：语义与远端版一致。
///
/// `Overwrite` 直接沿用原路径——`tokio::fs::File::create` 本身会截断既有文件。
///
/// R3-04：此处原先只拒绝空串/`.`/`..`，**漏掉分隔符检查**，与上面
/// `sibling_path` 的注释所声称的「远端版语义」并不一致，且让本地下载的
/// 「重命名」成为任意路径写入的旁路。现与远端版共用 [`is_unsafe_component`]，
/// 并额外要求解析结果仍位于原父目录内（纵深防御：驱动器相对名如 `C:x`
/// 能通过名字检查，但父目录会变）。
pub fn resolve_local_target(
    target: &std::path::Path,
    conflict: &ConflictPolicy,
) -> Result<PathBuf, CoreError> {
    if !target.exists() || matches!(conflict, ConflictPolicy::Overwrite) {
        return Ok(target.to_path_buf());
    }
    match conflict {
        ConflictPolicy::Overwrite => Ok(target.to_path_buf()),
        ConflictPolicy::Fail => Err(CoreError::TargetExists(format!(
            "本地已存在同名文件：{}",
            target.display()
        ))),
        ConflictPolicy::Rename(name) => {
            if is_unsafe_component(name) {
                return Err(CoreError::InvalidState(format!("Unsafe file name: {name}")));
            }
            let renamed = target.with_file_name(name);
            if renamed.parent() != target.parent() {
                return Err(CoreError::InvalidState(format!(
                    "重命名目标逃出了目标目录：{}",
                    renamed.display()
                )));
            }
            if renamed.exists() {
                return Err(CoreError::TargetExists(format!(
                    "重命名目标也已存在：{}",
                    renamed.display()
                )));
            }
            Ok(renamed)
        }
    }
}

/// 文件传输服务
pub struct TransferService {
    /// 传输任务队列
    tasks: Arc<RwLock<HashMap<Uuid, TransferTask>>>,
    /// 按 task_id 的传输控制通道：pause/resume/cancel 通过它驱动传输循环，
    /// cancel 与 pause 共用同一通道。任务进入终态后移除对应条目。
    control_channels: Arc<RwLock<HashMap<Uuid, watch::Sender<TransferControl>>>>,
    /// R2-T2：取消 / 提交竞态的决策 atomic —— `cancel_transfer` 在发送
    /// `TransferControl::Cancel` 同时把这里置 1，让 staged lifecycle 在
    /// commit 阶段 CAS 时看见。任务进入终态后移除对应条目。
    cancel_decisions: Arc<RwLock<HashMap<Uuid, Arc<AtomicU8>>>>,
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
            cancel_decisions: Arc::new(RwLock::new(HashMap::new())),
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

    /// 上传目标解析：只做纯路径计算，**不做存在性探测**。
    ///
    /// 探测需要开 SFTP 通道，会消耗 SSH provider 并打乱「provider 被调用时任务
    /// 已在表内且可控」这一已被 R2-01/R2-02 测试锁定的时序。改为由前端用已加载的
    /// 目录列表检测（体验即时），后端在 SFTP `create` 时强制（防 TOCTOU 竞态）。
    fn resolve_upload_target(remote: &str, conflict: &ConflictPolicy) -> Result<String, CoreError> {
        match conflict {
            ConflictPolicy::Overwrite => Ok(remote.to_string()),
            ConflictPolicy::Fail => Ok(remote.to_string()),
            ConflictPolicy::Rename(name) => {
                let renamed = sibling_path(remote, name)?;
                validate_remote_mutation_path(&renamed)?;
                Ok(renamed)
            }
        }
    }

    /// 添加上传任务并启动传输
    ///
    /// `conflict` 决定目标已存在时的行为。策略在**入队时**强制执行：即使前端
    /// 未先做存在性检查（TOCTOU），也不会出现静默覆盖。
    pub async fn enqueue_upload(
        &self,
        local: PathBuf,
        remote: String,
        session_id: Uuid,
        conflict: ConflictPolicy,
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
        let remote = Self::resolve_upload_target(&remote, &conflict)?;
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
            conflict: conflict.clone(),
            temp_path: None,
            cleanup_status: None,
            commit_status: None,
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
    ///
    /// `conflict` 决定本地目标已存在时的行为，语义同 [`Self::enqueue_upload`]。
    pub async fn enqueue_download(
        &self,
        remote: String,
        local: PathBuf,
        session_id: Uuid,
        conflict: ConflictPolicy,
    ) -> Result<Uuid, CoreError> {
        validate_remote_mutation_path(&remote)?;
        if !local.is_absolute() || local.file_name().is_none() {
            return Err(CoreError::InvalidState(
                "Download target must be an absolute local file".into(),
            ));
        }
        if !local.parent().is_some_and(|p| p.is_dir()) {
            return Err(CoreError::InvalidState(
                "Download target directory does not exist".into(),
            ));
        }
        let local = resolve_local_target(&local, &conflict)?;
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
            conflict: conflict.clone(),
            temp_path: None,
            cleanup_status: None,
            commit_status: None,
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

        // 建立 pause/resume/cancel 共用的控制通道 + 取消决策 atomic。
        //
        // 通道注册必须与启动决策在同一把 tasks 写锁临界区内完成：
        // cancel/pause 在各自的 tasks 写锁内读取通道（锁序恒为
        // tasks → control_channels），使「传输启动决策」与「取消/暂停决策」
        // 互斥——provider await 期间到达的取消/暂停不会因通道尚不存在而丢失：
        // 取消拦下启动，暂停则以 Pause 初值建通道，让循环首轮即挂起。
        let (mut control_rx, cancel_decision) = {
            let mut tasks = self.tasks.write().await;
            let Some(t) = tasks.get_mut(&task_id) else {
                return Ok(());
            };
            if t.state == TransferTaskState::Cancelled {
                // provider await 期间已被取消：不启动传输，保持 Cancelled 终态
                info!(task_id = %task_id, "Transfer cancelled during startup; not executing");
                return Ok(());
            }
            let paused_during_startup = t.state == TransferTaskState::Paused;
            let initial = if paused_during_startup {
                TransferControl::Pause
            } else {
                TransferControl::Run
            };
            let (control_tx, control_rx) = watch::channel(initial);
            self.control_channels
                .write()
                .await
                .insert(task_id, control_tx);
            // R2-T2：同时建立取消决策 atomic —— 让 cancel_transfer 在写
            // 控制通道的同时把这里置 1，staged lifecycle 在 copy 之后、
            // commit 之前看到。
            let cancel_decision = Arc::new(AtomicU8::new(DECISION_PENDING));
            self.cancel_decisions
                .write()
                .await
                .insert(task_id, cancel_decision.clone());
            // 顺手记录 temp_path —— 失败/取消的 residue 报告需要它。
            let temp = match task.direction {
                TransferDirection::Upload => {
                    staging_temp_path_str(&task.remote_path, &task_id.to_string())
                }
                TransferDirection::Download => {
                    staging_local_temp_path(&task.local_path, &task_id.to_string())
                        .to_string_lossy()
                        .into_owned()
                }
            };
            t.temp_path = Some(temp);
            (control_rx, cancel_decision)
        };

        // 启动异步传输任务
        let tasks = self.tasks.clone();
        let control_channels = self.control_channels.clone();
        let cancel_decisions = self.cancel_decisions.clone();
        let event_bus = self.event_bus.clone();
        tokio::spawn(async move {
            // R2-17：兜底守卫覆盖整个任务体——正常路径末尾 disarm；
            // 中途 panic 时由 Drop 兜底移除控制通道并写回失败终态。
            let mut cleanup_guard = TransferCleanupGuard {
                task_id,
                tasks: tasks.clone(),
                control_channels: control_channels.clone(),
                event_bus: event_bus.clone(),
                armed: true,
            };

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

            let outcome = Self::run_staged_transfer(
                &task,
                task_id,
                &ssh_client,
                &mut control_rx,
                &mut progress,
                cancel_decision,
            )
            .await;

            // 关闭发送端 → forwarder 排空最后一批进度后自行退出
            drop(progress);
            let _ = forwarder.await;

            // 清理控制通道：此后对该任务的 pause/resume/cancel 只改任务表状态
            {
                let mut controls = control_channels.write().await;
                controls.remove(&task_id);
            }
            // R2-T2：清理取消决策 atomic。
            {
                let mut decisions = cancel_decisions.write().await;
                decisions.remove(&task_id);
            }

            // 终态写回 —— 根据 staged outcome 把 task 写到 Completed / Failed /
            // Cancelled，并附上 commit 策略与 cleanup residue。
            Self::finalize_staged(&tasks, &event_bus, task_id, outcome).await;

            // 正常清理已全部完成：解除守卫，Drop 不再兜底
            cleanup_guard.disarm();
        });

        Ok(())
    }

    /// R2-T2：跑一次 staged transfer —— 包住「打开 SFTP / 选 sink / 跑
    /// upload 或 download staged 循环」。被 [`execute_transfer`] 的
    /// 异步任务体调用。
    async fn run_staged_transfer(
        task: &TransferTask,
        task_id: Uuid,
        ssh_client: &SshClientHandle,
        control_rx: &mut watch::Receiver<TransferControl>,
        progress: &mut (dyn FnMut(u64, u64) + Send),
        cancel_decision: Arc<AtomicU8>,
    ) -> StagedOutcome {
        // 启动窗口内到达的暂停/取消在此收口：循环尚未触碰远端，暂停挂起
        // 在打开 SFTP 通道之前，取消立即中止。
        if let Err(msg) = wait_for_run_before_start(control_rx).await {
            return StagedOutcome::Failed {
                stage: FailedStage::Copy,
                message: msg,
                residue_path: None,
            };
        }

        // 只在打开 SFTP 子通道时持有客户端读锁；SFTP 流独立于发送客户端，
        // 拷贝循环（含暂停挂起期间）不得阻塞会话断开等需要写锁的操作。
        let sftp_result = {
            let ssh = ssh_client.read().await;
            ssh.open_sftp_channel().await.map_err(|e| {
                ProtocolError::ProtocolError(format!("Failed to open SFTP channel: {e}"))
            })
        };
        let channel = match sftp_result {
            Ok(c) => c,
            Err(e) => {
                return StagedOutcome::Failed {
                    stage: FailedStage::ExclusiveOpen,
                    message: format!("{e}"),
                    residue_path: None,
                };
            }
        };
        let sftp = match SftpClient::new(channel).await {
            Ok(c) => c,
            Err(e) => {
                return StagedOutcome::Failed {
                    stage: FailedStage::ExclusiveOpen,
                    message: format!("{e}"),
                    residue_path: None,
                };
            }
        };
        let sink: Arc<dyn TransferSink> = Arc::new(sftp.as_sink());

        match task.direction {
            TransferDirection::Upload => {
                let source_total = match tokio::fs::metadata(&task.local_path).await {
                    Ok(m) => m.len(),
                    Err(e) => {
                        return StagedOutcome::Failed {
                            stage: FailedStage::Copy,
                            message: format!("local stat {}: {e}", task.local_path.display()),
                            residue_path: None,
                        };
                    }
                };
                let source = match tokio::fs::File::open(&task.local_path).await {
                    Ok(f) => f,
                    Err(e) => {
                        return StagedOutcome::Failed {
                            stage: FailedStage::Copy,
                            message: format!("local open {}: {e}", task.local_path.display()),
                            residue_path: None,
                        };
                    }
                };
                let temp_path = match task.temp_path_for(task_id) {
                    Some(t) => t,
                    None => {
                        return StagedOutcome::Failed {
                            stage: FailedStage::ExclusiveOpen,
                            message: "missing temp_path; enqueue must run first".into(),
                            residue_path: None,
                        };
                    }
                };
                run_staged_upload(UploadRequest {
                    source: Box::new(source),
                    source_total,
                    target: task.remote_path.clone(),
                    temp_path,
                    conflict: task.conflict.clone(),
                    sink,
                    control: control_rx,
                    progress,
                    cancel_decision,
                })
                .await
            }
            TransferDirection::Download => {
                let source_total = match sftp.metadata(&task.remote_path).await {
                    Ok(m) => m.size,
                    Err(e) => {
                        return StagedOutcome::Failed {
                            stage: FailedStage::Copy,
                            message: format!("remote stat {}: {e}", task.remote_path),
                            residue_path: None,
                        };
                    }
                };
                let source = match sink.open_read(&task.remote_path).await {
                    Ok(r) => r,
                    Err(e) => {
                        return StagedOutcome::Failed {
                            stage: FailedStage::Copy,
                            message: format!("remote open {}: {e}", task.remote_path),
                            residue_path: None,
                        };
                    }
                };
                let temp_path = match task.temp_path_for(task_id) {
                    Some(t) => t,
                    None => {
                        return StagedOutcome::Failed {
                            stage: FailedStage::ExclusiveOpen,
                            message: "missing temp_path; enqueue must run first".into(),
                            residue_path: None,
                        };
                    }
                };
                let local_sink: Arc<dyn TransferSink> = Arc::new(LocalSink::new());
                run_staged_download(DownloadRequest {
                    source,
                    source_total,
                    target: task.local_path.clone(),
                    temp_path: std::path::PathBuf::from(&temp_path),
                    conflict: task.conflict.clone(),
                    sink: local_sink,
                    control: control_rx,
                    progress,
                    cancel_decision,
                })
                .await
            }
        }
    }

    /// R2-T2：staged outcome → task 终态写回。
    ///
    /// 关键不变量：
    /// - Cancelled 终态不写 error_message（不是失败）。
    /// - Failed 终态必须保留原始错误 + cleanup residue。
    /// - Completed 必须带 commit 策略；residue 为 None（temp 已被 commit rename 取代）。
    async fn finalize_staged(
        tasks: &Arc<RwLock<HashMap<Uuid, TransferTask>>>,
        event_bus: &Arc<EventBus>,
        task_id: Uuid,
        outcome: StagedOutcome,
    ) {
        match outcome {
            StagedOutcome::Completed { bytes, commit } => {
                let mut tasks = tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    if t.state == TransferTaskState::Cancelled {
                        // commit 已在飞 / 完成后取消到达 —— commit 已生效，
                        // 终态是 Completed（取消意图被吞，因为目标已被替换）。
                        info!(
                            task_id = %task_id,
                            "Transfer commit succeeded after cancel request; keeping completed"
                        );
                        t.state = TransferTaskState::Completed;
                        t.finished_at = Some(std::time::Instant::now());
                        t.bytes_transferred = bytes;
                        t.commit_status = Some(CommitStatus::Committed(commit.strategy));
                        t.cleanup_status = Some(CleanupStatus::Cleaned);
                        event_bus.publish(AppEvent::TransferQueueChanged);
                    } else {
                        t.state = TransferTaskState::Completed;
                        t.finished_at = Some(std::time::Instant::now());
                        t.bytes_transferred = bytes;
                        t.commit_status = Some(CommitStatus::Committed(commit.strategy));
                        t.cleanup_status = Some(CleanupStatus::Cleaned);
                        event_bus.publish(AppEvent::TransferCompleted { task_id });
                        event_bus.publish(AppEvent::TransferQueueChanged);
                        info!(task_id = %task_id, strategy = ?commit.strategy, "Transfer completed");
                    }
                }
                Self::prune_finished_tasks(&mut tasks);
            }
            StagedOutcome::Cancelled {
                residue_path,
                cleanup_failure,
            } => {
                let mut tasks = tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    // 取消不是失败 —— 不写 error_message
                    t.cleanup_status = Some(match residue_path {
                        Some(path) => CleanupStatus::Residue {
                            path,
                            reason: cleanup_failure.unwrap_or_default(),
                        },
                        None => CleanupStatus::Cleaned,
                    });
                    info!(task_id = %task_id, residue = ?t.cleanup_status, "Transfer cancelled; staged cleanup done");
                    event_bus.publish(AppEvent::TransferQueueChanged);
                }
                Self::prune_finished_tasks(&mut tasks);
            }
            StagedOutcome::Failed {
                stage,
                message,
                residue_path,
            } => {
                let mut tasks = tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    if t.state == TransferTaskState::Cancelled {
                        // 取消 + commit 失败：保留 Cancelled 终态；residue 透出
                        t.cleanup_status = Some(match residue_path {
                            Some(path) => CleanupStatus::Residue {
                                path,
                                reason: message.clone(),
                            },
                            None => CleanupStatus::Cleaned,
                        });
                        info!(
                            task_id = %task_id,
                            stage = ?stage,
                            "Transfer commit failed after cancel; keeping cancelled, residue reported"
                        );
                        event_bus.publish(AppEvent::TransferQueueChanged);
                    } else {
                        t.state = TransferTaskState::Failed;
                        t.finished_at = Some(std::time::Instant::now());
                        // 把 stage 与原始 message 都透到 error_message，
                        // 方便前端按行展示：失败原因 + 残留路径
                        let full = format!("{}: {}", stage.label(), message);
                        t.error_message = Some(full);
                        t.cleanup_status = Some(match residue_path {
                            Some(path) => CleanupStatus::Residue {
                                path,
                                reason: message.clone(),
                            },
                            None => CleanupStatus::Cleaned,
                        });
                        // 冲突用固定前缀标记，前端据此提供「覆盖/重命名」入口
                        if let Some(path) = conflict_target(t.error_message.as_ref().unwrap()) {
                            event_bus.publish(AppEvent::TransferConflict {
                                task_id,
                                path: path.clone(),
                            });
                        }
                        event_bus.publish(AppEvent::TransferFailed {
                            task_id,
                            error: t.error_message.clone().unwrap_or_default(),
                        });
                        event_bus.publish(AppEvent::TransferQueueChanged);
                        warn!(task_id = %task_id, stage = ?stage, error = %message, "Transfer failed");
                    }
                }
                Self::prune_finished_tasks(&mut tasks);
            }
        }
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
                        t.finished_at = Some(std::time::Instant::now());
                        // 冲突用固定前缀标记，前端据此在队列行上提供「覆盖/重命名」
                        // 入口（重新入队时带上对应策略）。不用解析整句错误文案。
                        if let Some(path) = conflict_target(&describe_transfer_error(&e)) {
                            t.error_message = Some(format!("{TRANSFER_CONFLICT_PREFIX}{path}"));
                            event_bus.publish(AppEvent::TransferConflict {
                                task_id,
                                path: path.clone(),
                            });
                        } else {
                            t.error_message = Some(e.clone());
                        }
                        event_bus.publish(AppEvent::TransferFailed {
                            task_id,
                            error: t.error_message.clone().unwrap_or_default(),
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

    /// 移除传输任务条目（仅从队列列表消失，不触碰已传输的文件）
    ///
    /// 只允许移除**终态**（Completed/Failed/Cancelled）任务：活跃任务的拷贝
    /// 循环仍在跑并持有控制通道，提前移除会让 `apply_progress` 写不回任务表、
    /// 让控制通道条目泄漏。非终态调用返回 `CoreError::InvalidState`，前端
    /// 必须先取消再移除。
    ///
    /// 该操作只删内存中的队列条目，不删除本地或远端已传输的文件。
    pub async fn remove_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        match tasks.get(&task_id) {
            Some(task) if !task.state.is_terminal() => {
                return Err(CoreError::InvalidState(format!(
                    "transfer {task_id} is still {:?}; cancel it before removing",
                    task.state
                )));
            }
            Some(_) => {}
            None => {
                // 幂等：条目可能已被 prune_finished_tasks 限量清理移除。
                // 重复移除不视为错误，避免前端与后台清理竞态时弹错误。
                warn!(task_id = %task_id, "Remove requested for unknown transfer task");
                return Ok(());
            }
        }

        tasks.remove(&task_id);
        info!(task_id = %task_id, "Transfer task removed from queue");
        self.event_bus.publish(AppEvent::TransferQueueChanged);
        Ok(())
    }

    /// 暂停传输任务
    ///
    /// 置 Paused 并通过控制通道通知传输循环在下一分块前挂起；
    /// 字节随即停止增长，恢复后从已传字节继续。
    pub async fn pause_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        // 与 execute_transfer 的启动临界区互斥：先锁任务表、锁内再读控制通道
        // （锁序恒为 tasks → control_channels），通道注册前到达的暂停以
        // Pause 初值建通道收口，不会丢失。
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.state == TransferTaskState::Transferring {
                task.state = TransferTaskState::Paused;
                let control = self.control_channels.read().await.get(&task_id).cloned();
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
        // 与 pause_transfer 同序：tasks → control_channels
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.state == TransferTaskState::Paused {
                task.state = TransferTaskState::Transferring;
                let control = self.control_channels.read().await.get(&task_id).cloned();
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
    /// 仅非终态（Pending/Transferring/Paused）任务可取消：置 Cancelled 并通过
    /// 控制通道通知传输循环在下一分块前中止；循环结束后的终态写回会保持
    /// Cancelled，不广播 TransferCompleted。
    pub async fn cancel_transfer(&self, task_id: Uuid) -> Result<(), CoreError> {
        // 与 execute_transfer 的启动临界区互斥：通道注册前到达的取消直接
        // 拦下启动（execute_transfer 复查 Cancelled），不会丢信号。
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            // R2-07：终态（Completed/Failed/Cancelled）不得被改写——对 Failed
            // 任务取消会让 error_message 与 Cancelled 并存，重复取消还会重置
            // finished_at，均与 is_terminal「不会再发生状态变化」矛盾。
            if !task.state.is_terminal() {
                task.state = TransferTaskState::Cancelled;
                task.finished_at = Some(std::time::Instant::now());
                let control = self.control_channels.read().await.get(&task_id).cloned();
                if let Some(control) = control {
                    let _ = control.send(TransferControl::Cancel);
                }
                // R2-T2：同时把取消决策写入 atomic，让 staged lifecycle 在
                // copy → precheck 阶段看见（commit 还未发，commit CAS 会失败）。
                let decision = self.cancel_decisions.read().await.get(&task_id).cloned();
                if let Some(decision) = decision {
                    decision.store(DECISION_CANCELLED, std::sync::atomic::Ordering::SeqCst);
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

    /// R2-T2：从零重试终态任务 —— 创建新 task id，从原任务的
    /// session_id / source / target 重建，**强制**使用 `Fail` 策略
    /// （预检与提交之间存在时间窗，staged lifecycle 必须重新确认；
    /// 重试自动覆盖会绕过这条保护 —— ADR-0002 / spec §二）。
    ///
    /// 原任务保持它的终态——retry 创建的是全新 task id，不复用。
    /// 仅终态任务可重试；非终态（活跃 / 暂停）应先取消。
    pub async fn enqueue_retry(&self, task_id: Uuid) -> Result<Uuid, CoreError> {
        let source = {
            let tasks = self.tasks.read().await;
            tasks
                .get(&task_id)
                .ok_or_else(|| CoreError::NotFound(format!("Task {task_id} not found")))?
                .clone()
        };
        if !source.state.is_terminal() {
            return Err(CoreError::InvalidState(format!(
                "transfer {task_id} is still {:?}; cancel it before retry",
                source.state
            )));
        }
        let new_conflict = ConflictPolicy::Fail;
        match source.direction {
            TransferDirection::Upload => {
                self.enqueue_upload(
                    source.local_path.clone(),
                    source.remote_path.clone(),
                    source.session_id,
                    new_conflict,
                )
                .await
            }
            TransferDirection::Download => {
                self.enqueue_download(
                    source.remote_path.clone(),
                    source.local_path.clone(),
                    source.session_id,
                    new_conflict,
                )
                .await
            }
        }
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
    ///
    /// R2-21：仅非终态任务生效——终态（Completed/Failed/Cancelled）不得被改写。
    pub async fn mark_completed(&self, task_id: Uuid) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if !task.state.is_terminal() {
                task.state = TransferTaskState::Completed;
                task.finished_at = Some(std::time::Instant::now());
                info!(task_id = %task_id, "Transfer completed");
                self.event_bus
                    .publish(AppEvent::TransferCompleted { task_id });
                self.event_bus.publish(AppEvent::TransferQueueChanged);
            }
        }
        Self::prune_finished_tasks(&mut tasks);

        Ok(())
    }

    /// 标记传输失败
    ///
    /// R2-21：仅非终态任务生效——provider 出错路径若在启动窗口内的取消
    /// 之后到达，不得把已 Cancelled 的任务改写为 Failed、让取消意图被
    /// 终态覆盖（与 R2-07 对 cancel_transfer 的守卫同类）。
    pub async fn mark_failed(&self, task_id: Uuid, error: String) -> Result<(), CoreError> {
        let mut tasks = self.tasks.write().await;

        if let Some(task) = tasks.get_mut(&task_id) {
            if !task.state.is_terminal() {
                task.state = TransferTaskState::Failed;
                task.error_message = Some(error.clone());
                task.finished_at = Some(std::time::Instant::now());
                warn!(task_id = %task_id, error = %error, "Transfer failed");
                self.event_bus
                    .publish(AppEvent::TransferFailed { task_id, error });
                self.event_bus.publish(AppEvent::TransferQueueChanged);
            }
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

    // ── R2-T2：retry 入口回归 ──

    /// RED：retry 一个不存在的任务 → NotFound。
    #[tokio::test]
    async fn retry_rejects_unknown_task() {
        let svc = make_service();
        let err = svc
            .enqueue_retry(Uuid::new_v4())
            .await
            .expect_err("retry on unknown task must fail");
        assert!(matches!(err, CoreError::NotFound(_)), "实际: {err:?}");
    }

    /// RED：retry 非终态任务 → InvalidState。前端必须先取消再重试。
    #[tokio::test]
    async fn retry_rejects_non_terminal_task() {
        let svc = make_service();
        let id = Uuid::new_v4();
        insert_task(&svc, make_task(id, TransferTaskState::Transferring)).await;
        let err = svc
            .enqueue_retry(id)
            .await
            .expect_err("retry on active task must fail");
        assert!(matches!(err, CoreError::InvalidState(_)), "实际: {err:?}");
    }

    /// RED：retry 终态任务 → 强制 Fail 策略创建新任务；原任务保留终态不变。
    ///
    /// 注入「不阻塞、立刻返回假客户端」的 SSH provider —— 该客户端的
    /// `open_sftp_channel` 会失败并把新任务标记 Failed，但**入队**
    /// 这一段已经把我们关心的契约跑过了：策略为 Fail、路径与原任务相同、
    /// id 不同、原任务终态不变。
    #[tokio::test]
    async fn retry_creates_new_task_from_zero_with_fail_policy_and_keeps_original_terminal() {
        use rshell_api::types::{AuthMethod, Protocol as ApiProtocol, SessionConfig};
        use rshell_protocol::ssh::client::{ResolvedAuthMethod, SshClient};
        let svc = make_service();
        // 立刻返回假客户端 —— 不阻塞。
        svc.set_ssh_client_provider(Arc::new(|session_id| {
            let config = SessionConfig {
                id: session_id,
                name: "retry-test".into(),
                folder_id: None,
                host: "127.0.0.1".into(),
                port: 22,
                protocol: ApiProtocol::SSH,
                auth_method: AuthMethod::Password {
                    username: "u".into(),
                    has_password: true,
                },
                serial_config: None,
            };
            let auth = ResolvedAuthMethod::Password {
                username: "u".into(),
                password: "p".into(),
            };
            Box::pin(async move {
                Ok(
                    Arc::new(tokio::sync::RwLock::new(SshClient::new(config, auth)))
                        as crate::session::service::SshClientHandle,
                )
            })
        }));

        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("source.txt");
        std::fs::write(&local, b"payload").unwrap();
        std::mem::forget(dir);
        let id = Uuid::new_v4();
        let mut task = make_task(id, TransferTaskState::Failed);
        task.error_message = Some("boom".into());
        task.finished_at = Some(std::time::Instant::now());
        task.local_path = local.clone();
        task.remote_path = "/remote/source.txt".into();
        insert_task(&svc, task).await;

        let new_id = svc
            .enqueue_retry(id)
            .await
            .expect("retry on Failed task should succeed");
        assert_ne!(new_id, id, "retry 必须创建新 task id");

        // 原任务保留其 Failed 终态（spec：「retry 创建的是全新 task id」）
        let original = svc.get_task(id).await.unwrap();
        assert_eq!(original.state, TransferTaskState::Failed);
        assert_eq!(original.error_message.as_deref(), Some("boom"));

        // 新任务入队；execute_transfer 在假客户端上失败并把它标 Failed，
        // 但我们要看的契约是 conflict 策略、路径、id 不同。
        let new_task = svc.get_task(new_id).await.unwrap();
        assert!(matches!(new_task.conflict, ConflictPolicy::Fail));
        assert_eq!(new_task.local_path, original.local_path);
        assert_eq!(new_task.remote_path, original.remote_path);
    }

    #[tokio::test]
    async fn enqueue_upload_rejects_missing_file_before_queueing() {
        let service = make_service();
        let err = service
            .enqueue_upload(
                PathBuf::from("/definitely/missing/rshell-file"),
                "/remote/file".into(),
                Uuid::new_v4(),
                ConflictPolicy::Fail,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidState(_)));
        assert!(service.list_tasks().await.is_empty());
    }

    #[tokio::test]
    async fn enqueue_download_rejects_root_and_reports_existing_target() {
        let service = make_service();
        let folder = tempfile::tempdir().unwrap();
        let target = folder.path().join("file");
        assert!(matches!(
            service
                .enqueue_download(
                    "/".into(),
                    target.clone(),
                    Uuid::new_v4(),
                    ConflictPolicy::Fail
                )
                .await,
            Err(CoreError::InvalidState(_))
        ));
        std::fs::write(&target, b"keep").unwrap();
        // 目标已存在时不再是无差别 InvalidState，而是专门的 TargetExists：
        // 前端据此弹「覆盖 / 重命名 / 取消」，不会静默覆盖既有文件。
        assert!(matches!(
            service
                .enqueue_download(
                    "/remote/file".into(),
                    target.clone(),
                    Uuid::new_v4(),
                    ConflictPolicy::Fail
                )
                .await,
            Err(CoreError::TargetExists(_))
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
            conflict: ConflictPolicy::Fail,
            temp_path: None,
            cleanup_status: None,
            commit_status: None,
        }
    }

    #[tokio::test]
    async fn remove_transfer_deletes_terminal_task_from_queue() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            let mut task = make_task(id, TransferTaskState::Completed);
            task.finished_at = Some(std::time::Instant::now());
            tasks.insert(id, task);
        }

        svc.remove_transfer(id).await.unwrap();

        assert!(
            svc.list_tasks().await.is_empty(),
            "终态任务移除后不得再出现在队列"
        );
    }

    #[tokio::test]
    async fn remove_transfer_rejects_non_terminal_task() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Transferring));
        }

        let err = svc.remove_transfer(id).await.unwrap_err();
        assert!(
            matches!(err, CoreError::InvalidState(_)),
            "活跃任务不得被直接移除，须先取消"
        );
        assert!(
            svc.get_task(id).await.is_some(),
            "被拒绝的移除不得影响任务表"
        );
    }

    #[tokio::test]
    async fn remove_transfer_rejects_paused_task() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Paused));
        }

        assert!(matches!(
            svc.remove_transfer(id).await.unwrap_err(),
            CoreError::InvalidState(_)
        ));
        assert!(svc.get_task(id).await.is_some());
    }

    #[tokio::test]
    async fn remove_transfer_is_idempotent_for_unknown_task() {
        let svc = make_service();
        // 未知/已被限量清理移除的条目重复移除不应报错，
        // 否则前端与 prune_finished_tasks 竞态时会弹出无意义错误。
        svc.remove_transfer(Uuid::new_v4()).await.unwrap();
        svc.remove_transfer(Uuid::new_v4()).await.unwrap();
    }

    #[tokio::test]
    async fn remove_transfer_publishes_queue_change() {
        let event_bus = Arc::new(EventBus::new());
        let svc = TransferService::new(event_bus.clone());
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            let mut task = make_task(id, TransferTaskState::Cancelled);
            task.finished_at = Some(std::time::Instant::now());
            tasks.insert(id, task);
        }

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        event_bus.subscribe(move |event| {
            if let AppEvent::TransferQueueChanged = event {
                sink.lock().unwrap().push(());
            }
        });

        svc.remove_transfer(id).await.unwrap();
        // 事件总线异步投递，留出调度时间
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(
            !seen.lock().unwrap().is_empty(),
            "移除条目后必须广播 TransferQueueChanged 让前端刷新"
        );
    }

    // ── 覆盖冲突策略 ──
    #[test]
    fn sibling_path_keeps_directory_and_rejects_escaping_names() {
        assert_eq!(
            sibling_path("/home/u/a.txt", "b.txt").unwrap(),
            "/home/u/b.txt"
        );
        assert_eq!(sibling_path("/a.txt", "b.txt").unwrap(), "/b.txt");
        // 名字含分隔符必须被拒——否则「重命名」会成为任意路径写入的旁路
        for bad in ["", ".", "..", "a/b", "a\\b", "   ", "a\0b"] {
            assert!(
                sibling_path("/home/u/a.txt", bad).is_err(),
                "name={bad:?} 应被拒绝"
            );
        }
        // 没有父目录的裸名无法推导目录
        assert!(sibling_path("bare.txt", "b.txt").is_err());
    }

    #[tokio::test]
    async fn resolve_local_target_reports_existing_and_honours_policy() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("a.txt");
        std::fs::write(&existing, b"old").unwrap();

        // Fail：已存在即报错，供前端弹覆盖/重命名对话框
        let err = resolve_local_target(&existing, &ConflictPolicy::Fail).unwrap_err();
        assert!(matches!(err, CoreError::TargetExists(_)), "实际: {err:?}");

        // Overwrite：沿用原路径（后续 File::create 会截断）
        assert_eq!(
            resolve_local_target(&existing, &ConflictPolicy::Overwrite).unwrap(),
            existing
        );

        // Rename：产出同目录下的新名，且必须尚不存在
        let renamed =
            resolve_local_target(&existing, &ConflictPolicy::Rename("b.txt".into())).unwrap();
        assert_eq!(renamed, dir.path().join("b.txt"));
        // 新名也被占用时必须报错，否则「重命名」退化为又一次静默覆盖
        std::fs::write(&renamed, b"x").unwrap();
        assert!(matches!(
            resolve_local_target(&existing, &ConflictPolicy::Rename("b.txt".into())).unwrap_err(),
            CoreError::TargetExists(_)
        ));
    }

    /// 回归 R3-04：本地下载的「重命名」不得成为任意路径写入的旁路。
    ///
    /// 旧实现只拒绝空串/`.`/`..`，不检查分隔符，而 `Path::with_file_name`
    /// 不做净化 —— 含 `../`、`\`、驱动器前缀的名字会让下载落到目标目录之外。
    /// 远端孪生实现 `sibling_path` 早已逐项拒绝，本地版此前漏了这层。
    #[test]
    fn local_rename_rejects_names_that_escape_the_destination_directory() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("a.txt");
        std::fs::write(&existing, b"old").unwrap();

        for hostile in [
            "../evil.txt",      // POSIX 上一层
            "../../evil.txt",   // POSIX 多层
            "sub/evil.txt",     // POSIX 子目录
            "..\\evil.txt",     // Windows 上一层
            "..\\..\\evil.txt", // Windows 多层
            "sub\\evil.txt",    // Windows 子目录
            "",
            ".",
            "..",
            "   ",
        ] {
            let err = resolve_local_target(&existing, &ConflictPolicy::Rename(hostile.into()))
                .expect_err(&format!("改名 {hostile:?} 必须被拒绝"));
            assert!(
                matches!(err, CoreError::InvalidState(_)),
                "改名 {hostile:?} 应报 InvalidState，实际: {err:?}"
            );
        }

        // 驱动器相对名 `C:evil.txt` 的语义因平台而异：Windows 上
        // `Path::with_file_name` 会把整条路径替换成驱动器相对路径，从而逃出目标
        // 目录，必须被拒；POSIX 上 `:` 只是普通文件名字符，结果仍落在目标目录内，
        // 放行并不构成旁路。两条分支都断言同一个跨平台不变式——不得逃出目录，
        // 而不是断言某个平台专属的报错。
        match resolve_local_target(&existing, &ConflictPolicy::Rename("C:evil.txt".into())) {
            Err(err) => assert!(
                matches!(err, CoreError::InvalidState(_)),
                "改名 \"C:evil.txt\" 被拒时应报 InvalidState，实际: {err:?}"
            ),
            Ok(path) => assert_eq!(
                path.parent(),
                existing.parent(),
                "改名 \"C:evil.txt\" 放行时必须仍在原父目录内，实际: {}",
                path.display()
            ),
        }
    }

    /// 回归 R3-04（配套）：合法的单分量改名仍须放行，且不越出目标目录。
    #[test]
    fn local_rename_accepts_a_plain_component_staying_in_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("a.txt");
        std::fs::write(&existing, b"old").unwrap();

        let renamed =
            resolve_local_target(&existing, &ConflictPolicy::Rename("b.txt".into())).unwrap();
        assert_eq!(renamed, dir.path().join("b.txt"));
        assert_eq!(
            renamed.parent(),
            existing.parent(),
            "改名结果必须仍在原父目录内"
        );

        // 远端版与本地版对同一个名字必须给出同样的裁决（语义一致）。
        assert!(sibling_path("/home/user/a.txt", "../evil.txt").is_err());
        assert!(sibling_path("/home/user/a.txt", "b.txt").is_ok());
    }

    #[tokio::test]
    async fn resolve_local_target_passes_through_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = dir.path().join("new.txt");
        for policy in [
            ConflictPolicy::Fail,
            ConflictPolicy::Overwrite,
            ConflictPolicy::Rename("other.txt".into()),
        ] {
            assert_eq!(resolve_local_target(&fresh, &policy).unwrap(), fresh);
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

    // ===== R2-07：终态不得被取消改写，非终态取消行为不变 =====

    /// 验收：对 Failed 任务调用 cancel_transfer 后状态仍为 Failed——
    /// error_message 与终态不被改写，也不广播队列刷新。
    #[tokio::test]
    async fn cancel_failed_task_is_noop() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            let mut task = make_task(id, TransferTaskState::Failed);
            task.error_message = Some("disk full".to_string());
            task.finished_at = Some(std::time::Instant::now());
            tasks.insert(id, task);
        }
        let seen = collect_events(&svc);

        svc.cancel_transfer(id).await.unwrap();

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(
            t.state,
            TransferTaskState::Failed,
            "终态 Failed 不得被改写为 Cancelled"
        );
        assert_eq!(
            t.error_message.as_deref(),
            Some("disk full"),
            "错误信息必须保留"
        );
        assert!(
            seen.lock().unwrap().is_empty(),
            "终态任务的取消不得广播任何事件"
        );
    }

    // ===== R2-17：兜底清理守卫 =====

    /// 验收：任务体 unwind（panic）路径下，Drop 守卫兜底移除 control_channels
    /// 条目并把任务写回终态——否则幽灵任务永久占用任务表与控制通道。
    #[tokio::test]
    async fn cleanup_guard_removes_channel_and_writes_terminal_state_on_unwind() {
        let svc = Arc::new(make_service());
        let id = Uuid::new_v4();
        insert_task(&svc, make_task(id, TransferTaskState::Transferring)).await;
        let mut rx = register_control(&svc, id).await;
        let seen = collect_events(&svc);

        {
            // armed 状态下析构 = 模拟任务体中途 panic 的 unwind 路径
            let guard = TransferCleanupGuard {
                task_id: id,
                tasks: svc.tasks.clone(),
                control_channels: svc.control_channels.clone(),
                event_bus: svc.event_bus.clone(),
                armed: true,
            };
            drop(guard);
        }

        // Drop 内 spawn 的后台清理需要调度时间
        for _ in 0..200 {
            if svc.control_channels.read().await.get(&id).is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        assert!(
            svc.control_channels.read().await.get(&id).is_none(),
            "控制通道条目必须被移除"
        );
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Failed, "终态必须被写回");
        assert_eq!(
            t.error_message.as_deref(),
            Some("transfer task panicked before finishing")
        );
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, AppEvent::TransferFailed { task_id, .. } if *task_id == id)),
            "兜底写回必须广播 TransferFailed"
        );
        // 发送端已移除：等待控制信号会失败
        assert!(rx.changed().await.is_err());
    }

    /// 正常路径 disarm 后 Drop 不再兜底：不覆盖已写回的终态、不重复清理
    #[tokio::test]
    async fn cleanup_guard_disarmed_drop_is_noop() {
        let svc = Arc::new(make_service());
        let id = Uuid::new_v4();
        insert_task(&svc, make_task(id, TransferTaskState::Transferring)).await;
        register_control(&svc, id).await;
        let seen = collect_events(&svc);

        {
            let mut guard = TransferCleanupGuard {
                task_id: id,
                tasks: svc.tasks.clone(),
                control_channels: svc.control_channels.clone(),
                event_bus: svc.event_bus.clone(),
                armed: true,
            };
            // 正常路径动作：移除控制通道 + 写回终态，然后 disarm
            svc.control_channels.write().await.remove(&id);
            svc.mark_completed(id).await.unwrap();
            guard.disarm();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Completed, "终态不得被兜底覆盖");
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, AppEvent::TransferFailed { .. })),
            "disarm 后不得再广播 TransferFailed"
        );
        assert!(svc.control_channels.read().await.get(&id).is_none());
    }

    /// 验收：非终态（Pending）取消行为不变
    #[tokio::test]
    async fn test_cancel_pending_task_moves_to_cancelled() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Pending));
        }

        svc.cancel_transfer(id).await.unwrap();
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Cancelled);
    }

    /// 验收：非终态（Paused）取消行为不变，且控制通道收到 Cancel
    #[tokio::test]
    async fn test_cancel_paused_task_moves_to_cancelled() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            tasks.insert(id, make_task(id, TransferTaskState::Paused));
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

    // ===== R2-21：mark_failed/mark_completed 不得改写终态 =====

    /// 验收：对已 Cancelled 任务调用 mark_failed 后状态仍为 Cancelled，
    /// 不写错误信息、不广播 TransferFailed。
    #[tokio::test]
    async fn mark_failed_keeps_cancelled_terminal_state() {
        let svc = make_service();
        let id = Uuid::new_v4();
        insert_task(&svc, make_task(id, TransferTaskState::Cancelled)).await;
        let seen = collect_events(&svc);

        svc.mark_failed(id, "provider blew up after cancel".to_string())
            .await
            .unwrap();

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(
            t.state,
            TransferTaskState::Cancelled,
            "取消终态不得被 mark_failed 改写为 Failed"
        );
        assert!(t.error_message.is_none(), "取消终态不得被写入错误信息");
        assert!(seen.lock().unwrap().is_empty(), "终态不得再广播任何事件");
    }

    /// 同类守卫：mark_completed 也不得改写终态
    #[tokio::test]
    async fn mark_completed_keeps_failed_terminal_state() {
        let svc = make_service();
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            let mut task = make_task(id, TransferTaskState::Failed);
            task.error_message = Some("disk full".to_string());
            tasks.insert(id, task);
        }
        let seen = collect_events(&svc);

        svc.mark_completed(id).await.unwrap();

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(t.state, TransferTaskState::Failed, "失败终态不得被改写");
        assert_eq!(t.error_message.as_deref(), Some("disk full"));
        assert!(seen.lock().unwrap().is_empty(), "终态不得再广播任何事件");
    }

    /// 验收：非终态任务的失败路径行为不变（Pending/Transferring/Paused 均可标记失败）
    #[tokio::test]
    async fn mark_failed_still_applies_to_non_terminal_states() {
        for state in [
            TransferTaskState::Pending,
            TransferTaskState::Transferring,
            TransferTaskState::Paused,
        ] {
            let svc = make_service();
            let id = Uuid::new_v4();
            insert_task(&svc, make_task(id, state)).await;
            let seen = collect_events(&svc);

            svc.mark_failed(id, "boom".to_string()).await.unwrap();

            let t = svc.get_task(id).await.unwrap();
            assert_eq!(t.state, TransferTaskState::Failed);
            assert_eq!(t.error_message.as_deref(), Some("boom"));
            assert!(seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, AppEvent::TransferFailed { task_id, .. } if *task_id == id)));
        }
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
            conflict: ConflictPolicy::Fail,
            temp_path: None,
            cleanup_status: None,
            commit_status: None,
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
            conflict: ConflictPolicy::Fail,
            temp_path: None,
            cleanup_status: None,
            commit_status: None,
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

    // ===== R2-01：provider await 窗口内的取消/暂停不得丢失 =====

    use rshell_api::types::{AuthMethod, Protocol as ApiProtocol, SessionConfig};
    use rshell_protocol::ssh::client::{ResolvedAuthMethod, SshClient};
    use std::future::Future;

    /// 可控 SSH provider：被调用时先通知 `entered`（测试据此确认已进入
    /// provider await 窗口），随后阻塞在 `gate` 上，放行后返回一个未连接的
    /// SshClient——其 open_sftp_channel 立即失败，可证明循环一旦真正启动
    /// 必然触发终态写回。
    fn gated_provider(
        entered: Arc<tokio::sync::Notify>,
        gate: Arc<tokio::sync::Notify>,
    ) -> SshClientProvider {
        Arc::new(move |_session_id: Uuid| {
            let entered = entered.clone();
            let gate = gate.clone();
            Box::pin(async move {
                entered.notify_one();
                gate.notified().await;
                let config = SessionConfig {
                    id: Uuid::new_v4(),
                    name: "race-test".into(),
                    folder_id: None,
                    host: "127.0.0.1".into(),
                    port: 22,
                    protocol: ApiProtocol::SSH,
                    auth_method: AuthMethod::Password {
                        username: "u".into(),
                        has_password: true,
                    },
                    serial_config: None,
                };
                let auth = ResolvedAuthMethod::Password {
                    username: "u".into(),
                    password: "p".into(),
                };
                Ok(
                    Arc::new(tokio::sync::RwLock::new(SshClient::new(config, auth)))
                        as SshClientHandle,
                )
            })
                as Pin<Box<dyn Future<Output = Result<SshClientHandle, CoreError>> + Send>>
        })
    }

    /// 入队一个上传任务并等到 provider await 窗口内（provider 已被调用、
    /// 正阻塞在 gate 上），返回 (任务 id, gate, 入队 future 句柄)。
    async fn enqueue_into_provider_window(
        svc: &Arc<TransferService>,
    ) -> (
        Uuid,
        Arc<tokio::sync::Notify>,
        tokio::task::JoinHandle<Result<Uuid, CoreError>>,
    ) {
        let entered = Arc::new(tokio::sync::Notify::new());
        let gate = Arc::new(tokio::sync::Notify::new());
        svc.set_ssh_client_provider(gated_provider(entered.clone(), gate.clone()));

        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("source.txt");
        std::fs::write(&local, b"payload").unwrap();
        // tempdir 在本函数返回时会被删除导致路径失效：泄漏目录，
        // 由测试进程退出统一回收（体量仅几字节）。
        std::mem::forget(dir);

        let enqueue = {
            let svc = svc.clone();
            tokio::spawn(async move {
                svc.enqueue_upload(
                    local,
                    "/remote/file".into(),
                    Uuid::new_v4(),
                    ConflictPolicy::Fail,
                )
                .await
            })
        };
        // provider 已被调用 ⇒ 任务已置 Transferring、正卡在 await 窗口内
        entered.notified().await;
        let id = svc
            .list_tasks()
            .await
            .first()
            .expect("任务在进入 await 窗口前必须已入队")
            .id;
        (id, gate, enqueue)
    }

    /// 验收：await 窗口内的取消不得丢失——传输循环不得启动，终态保持
    /// Cancelled 且与实际一致（本地文件未动、无完成/失败事件）。
    #[tokio::test]
    async fn cancel_during_provider_await_prevents_transfer_start() {
        let svc = Arc::new(make_service());
        let seen = collect_events(&svc);
        let (id, gate, enqueue) = enqueue_into_provider_window(&svc).await;

        // 窗口内取消：此刻控制通道尚未注册，取消不得因此丢失
        svc.cancel_transfer(id).await.unwrap();
        assert_eq!(
            svc.get_task(id).await.unwrap().state,
            TransferTaskState::Cancelled
        );

        // 放行 provider：execute_transfer 必须在同一把 tasks 写锁内复查
        // Cancelled 并拦下启动（回归时这里会注册 Run 通道并 spawn 循环）
        gate.notify_one();
        enqueue.await.unwrap().expect("启动被拦下应返回 Ok");

        // 给调度器留出时间：若回归（循环被启动），假客户端失败后会终态写回
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let t = svc.get_task(id).await.unwrap();
        assert_eq!(
            t.state,
            TransferTaskState::Cancelled,
            "取消终态不得被传输循环的写回改写"
        );
        assert!(t.error_message.is_none(), "取消不是失败，不得写入错误信息");
        assert!(
            !seen.lock().unwrap().iter().any(|e| matches!(
                e,
                AppEvent::TransferCompleted { .. } | AppEvent::TransferFailed { .. }
            )),
            "未启动的传输不得广播完成/失败事件"
        );
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|e| matches!(e, AppEvent::TransferQueueChanged))
                .count(),
            2,
            "只应有入队与取消两次队列刷新；若回归（循环启动后写回终态）会是 3 次"
        );
    }

    /// 验收：await 窗口内的暂停不得丢失——控制通道以 Pause 初值建立，
    /// 循环在打开 SFTP 通道前挂起，任务保持 Paused 等待恢复；恢复后循环
    /// 真正前进（在假客户端上以失败收尾，证明挂起可恢复）。
    #[tokio::test]
    async fn pause_during_provider_await_suspends_loop_before_remote_contact() {
        let svc = Arc::new(make_service());
        let seen = collect_events(&svc);
        let (id, gate, enqueue) = enqueue_into_provider_window(&svc).await;

        // 窗口内暂停：此刻控制通道尚未注册，暂停不得因此丢失
        svc.pause_transfer(id).await.unwrap();
        assert_eq!(
            svc.get_task(id).await.unwrap().state,
            TransferTaskState::Paused
        );

        // 放行 provider：循环以 Pause 初值启动，必须在打开 SFTP 通道前挂起
        gate.notify_one();
        enqueue.await.unwrap().expect("启动照常返回 Ok");

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(
            t.state,
            TransferTaskState::Paused,
            "启动窗口内的暂停必须生效；回归时循环会照跑并在假客户端上失败改写为 Failed"
        );
        assert!(t.error_message.is_none(), "挂起中的任务不得写入错误信息");
        assert!(
            svc.control_channels.read().await.contains_key(&id),
            "挂起中的循环必须仍持有控制通道等待恢复"
        );
        assert!(
            !seen.lock().unwrap().iter().any(|e| matches!(
                e,
                AppEvent::TransferCompleted { .. } | AppEvent::TransferFailed { .. }
            )),
            "挂起中的传输不得广播完成/失败事件"
        );

        // 恢复后循环真正前进：假客户端打不开 SFTP 通道 → 按失败收尾
        svc.resume_transfer(id).await.unwrap();
        for _ in 0..200 {
            if svc.get_task(id).await.unwrap().state.is_terminal() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let t = svc.get_task(id).await.unwrap();
        assert_eq!(
            t.state,
            TransferTaskState::Failed,
            "恢复后挂起的循环应继续执行（假客户端上以失败收尾）"
        );
        assert!(t.error_message.is_some(), "失败收尾必须携带错误信息");
    }
}
