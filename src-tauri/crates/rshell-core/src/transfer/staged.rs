//! R2-T2：staged 传输生命周期 —— 写临时文件、提交、清理。
//!
//! 这是 R2-T2 的核心：core 传输服务的执行循环委托给本模块的 [`run_staged_upload`]
//! / [`run_staged_download`]，把「打开 temp → 拷贝 → 重新检查冲突 → 提交 → 失败/取消
//! 时清理」这一串步骤集中起来，便于在测试里用一个 in-memory 的 fake sink 全程驱动。
//!
//! 取消 / 提交竞态在这里仲裁：见 `cancel_decision` 的注释。

use rshell_api::types::ConflictPolicy;
use rshell_protocol::ssh::sftp::{CommitOutcome, CommitStrategy, TransferSink};
use rshell_protocol::ProtocolError;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

/// 取消 / 提交竞态的最终决策编码。
///
/// - `0`（默认）：无人在裁决点抢先。
/// - `1`：取消已被记录，下游必须短路 commit 并保留目标。
/// - `2`：提交已被记录，下游必须把任务写成 `Completed`，取消意图被吞。
///
/// `cancel_transfer` 写 `1`；`safe_commit` 成功后用 CAS(0→2) 抢占决策。
/// 这套机制保证「已取消却替换目标」或「已提交却报告取消」两种错误状态都不出现。
pub const DECISION_PENDING: u8 = 0;
pub const DECISION_CANCELLED: u8 = 1;
pub const DECISION_COMMITTED: u8 = 2;

/// 临时文件路径生成 —— 在 `target` 同目录下加 `.partial-<task_uuid>` 后缀。
///
/// 同目录是为了让最终 rename 在所有目标平台（macOS / Linux / Windows）上
/// 都是同文件系统内 rename —— 跨目录 rename 在某些远端 / 文件系统上
/// 可能不是原子替换。
///
/// 「根目录文件」如 `/a.txt`：斜杠在 0 位置，父目录就是根，name 是 `a.txt`，
/// 拼接结果必须是 `/a.txt.partial-<uuid>`，不能多一个 `/`。
pub fn staging_temp_path_str(target: &str, task_uuid: &str) -> String {
    match target.rfind('/') {
        Some(0) => format!("/{}.partial-{task_uuid}", &target[1..]),
        Some(slash) => format!(
            "{}/{}.partial-{task_uuid}",
            &target[..slash],
            &target[slash + 1..]
        ),
        None => format!("{target}.partial-{task_uuid}"),
    }
}

/// 本地下载的临时文件路径 —— 在 `target` 同目录下加 `.partial-<task_uuid>` 后缀。
///
/// `Path::file_name` + 拼回父目录，避免跨目录 / 越界。
pub fn staging_local_temp_path(target: &Path, task_uuid: &str) -> std::path::PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("transfer");
    parent.join(format!("{name}.partial-{task_uuid}"))
}

/// Staged 传输的最终结果 —— 上层按此写终态、residue、commit 策略。
#[derive(Debug)]
pub enum StagedOutcome {
    /// 拷贝 + 提交都成功。`commit.strategy` 用于透传到前端，让用户知道
    /// 是否拿到了原子替换保证。
    Completed { bytes: u64, commit: CommitOutcome },
    /// 取消仲裁触发了：commit 未发送、目标未变。`residue_path` 仅在
    /// 「清理 temp 也失败」时给出，调用方把它当作 residue 报告。
    Cancelled {
        residue_path: Option<String>,
        cleanup_failure: Option<String>,
    },
    /// 拷贝、提交或清理失败 —— 旧目标保持不变（前提：拷贝或提交失败；
    /// 提交成功后写失败仅指 cleanup，无法回滚目标替换）。
    Failed {
        stage: FailedStage,
        message: String,
        /// 若清理后 temp 仍存在，则如实告诉调用方 —— 不可静默吞掉
        residue_path: Option<String>,
    },
}

/// 失败发生的位置 —— 决定任务终态与错误信息前缀。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedStage {
    /// 独占打开 temp 失败（O_EXCL 撞同 UUID 或权限不足）。
    ExclusiveOpen,
    /// 字节拷贝失败（读 / 写 / 取消中途到达）。
    Copy,
    /// 提交前再次检查目标，发现目标状态与原策略不符（晚冲突）。
    LateConflict,
    /// 提交（rename）失败 —— temp 可能残留。
    Commit,
    /// 拷贝成功后取消仲裁触发了：commit 不会执行。
    CancelledBeforeCommit,
}

/// 上传方向：源是 `AsyncRead`（本地文件），temp 与 target 在远端。
pub struct UploadRequest<'a> {
    pub source: Box<dyn AsyncRead + Unpin + Send>,
    pub source_total: u64,
    pub target: String,
    pub temp_path: String,
    pub conflict: ConflictPolicy,
    pub sink: Arc<dyn TransferSink>,
    pub control: &'a mut watch::Receiver<rshell_protocol::ssh::sftp::TransferControl>,
    /// 进度回调必须 `Send` —— 上层会把整个 staged 调用放进 `tokio::spawn` 里。
    pub progress: &'a mut (dyn FnMut(u64, u64) + Send),
    pub cancel_decision: Arc<AtomicU8>,
}

/// 下载方向：源是远端（在 sink 上打开），temp 与 target 在本地。
pub struct DownloadRequest<'a> {
    pub source: Box<dyn AsyncRead + Unpin + Send>,
    pub source_total: u64,
    pub target: std::path::PathBuf,
    pub temp_path: std::path::PathBuf,
    pub conflict: ConflictPolicy,
    pub sink: Arc<dyn TransferSink>,
    pub control: &'a mut watch::Receiver<rshell_protocol::ssh::sftp::TransferControl>,
    /// 进度回调必须 `Send` —— 上层会把整个 staged 调用放进 `tokio::spawn` 里。
    pub progress: &'a mut (dyn FnMut(u64, u64) + Send),
    pub cancel_decision: Arc<AtomicU8>,
}

/// 跑上传方向的 staged lifecycle。
pub async fn run_staged_upload(mut req: UploadRequest<'_>) -> StagedOutcome {
    let target = req.target.clone();
    let temp_path = req.temp_path.clone();

    // 1) 独占打开 temp
    let mut sink = match req.sink.exclusive_open_write(&temp_path).await {
        Ok(s) => s,
        Err(e) => {
            return StagedOutcome::Failed {
                stage: FailedStage::ExclusiveOpen,
                message: format!("exclusive open {temp_path}: {e}"),
                residue_path: None,
            };
        }
    };

    // 2) 字节拷贝（拷贝循环内部已检查 control）
    let copy_result = copy_bytes(
        &mut req.source,
        sink.as_mut(),
        req.source_total,
        req.control,
        req.progress,
    )
    .await;

    // 取消 / IO 错误：先尝试清理 temp；按 cancel_decision 决定终态
    let bytes = match copy_result {
        Ok(bytes) => bytes,
        Err(e) => {
            let cleanup = req.sink.try_remove(&temp_path).await;
            let cleanup_failure = match cleanup {
                Ok(()) => None,
                Err(ProtocolError::TransferCleanupFailed { path, reason }) => {
                    Some(format!("{path}: {reason}"))
                }
                Err(other) => Some(format!("{other}")),
            };
            return StagedOutcome::Failed {
                stage: FailedStage::Copy,
                message: format!("copy: {e}"),
                residue_path: cleanup_failure.is_some().then(|| temp_path.clone()),
            };
        }
    };

    // 3) 拷贝成功 → 检查取消决策
    if req.cancel_decision.load(Ordering::SeqCst) == DECISION_CANCELLED {
        // 取消抢先到达：commit 不发送、目标保留；清理 temp
        let cleanup = req.sink.try_remove(&temp_path).await;
        let cleanup_failure = match cleanup {
            Ok(()) => None,
            Err(ProtocolError::TransferCleanupFailed { path, reason }) => {
                Some(format!("{path}: {reason}"))
            }
            Err(other) => Some(format!("{other}")),
        };
        return StagedOutcome::Cancelled {
            residue_path: cleanup_failure.is_some().then(|| temp_path.clone()),
            cleanup_failure,
        };
    }

    // 4) 提交前再次检查目标是否存在 —— 应对预检后冲突
    let target_existed = req.sink.exists(&target).await;
    if target_existed && !matches!(req.conflict, ConflictPolicy::Overwrite) {
        // 预检通过后目标出现 —— 报告晚冲突并清理 temp
        let cleanup = req.sink.try_remove(&temp_path).await;
        let cleanup_failure = match cleanup {
            Ok(()) => None,
            Err(ProtocolError::TransferCleanupFailed { path, reason }) => {
                Some(format!("{path}: {reason}"))
            }
            Err(other) => Some(format!("{other}")),
        };
        return StagedOutcome::Failed {
            stage: FailedStage::LateConflict,
            message: format!("target appeared before commit: {target}"),
            residue_path: cleanup_failure.is_some().then(|| temp_path.clone()),
        };
    }

    // 5) 提交 —— capabilities 探测目前固定返回 default；保留接口以备后续升级
    let caps = rshell_protocol::ssh::sftp::SftpCapabilities::default();
    let commit = match req.sink.safe_commit(&temp_path, &target, caps).await {
        Ok(c) => c,
        Err(e) => {
            let cleanup = req.sink.try_remove(&temp_path).await;
            let cleanup_failure = match cleanup {
                Ok(()) => None,
                Err(ProtocolError::TransferCleanupFailed { path, reason }) => {
                    Some(format!("{path}: {reason}"))
                }
                Err(other) => Some(format!("{other}")),
            };
            let residue = if cleanup_failure.is_some() {
                Some(temp_path.clone())
            } else {
                None
            };
            return StagedOutcome::Failed {
                stage: FailedStage::Commit,
                message: format!("commit {target}: {e}"),
                residue_path: residue,
            };
        }
    };

    // 6) 提交成功 → 抢占决策点
    match req.cancel_decision.compare_exchange(
        DECISION_PENDING,
        DECISION_COMMITTED,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => StagedOutcome::Completed { bytes, commit },
        Err(DECISION_CANCELLED) => {
            // 取消在 commit 期间到达 —— commit 已生效；任务终态是 Completed
            // （取消意图被吞，因为目标已被原子替换）。residue 已无意义。
            StagedOutcome::Completed { bytes, commit }
        }
        Err(other) => {
            unreachable!("unexpected decision value: {other}")
        }
    }
}

/// 跑下载方向的 staged lifecycle —— 语义与上传对称。
pub async fn run_staged_download(mut req: DownloadRequest<'_>) -> StagedOutcome {
    let target = req.target.clone();
    let temp_path = req.temp_path.clone();
    let temp_str = temp_path.to_string_lossy().into_owned();

    // 1) 独占打开 temp
    let mut sink = match req.sink.exclusive_open_write(&temp_str).await {
        Ok(s) => s,
        Err(e) => {
            return StagedOutcome::Failed {
                stage: FailedStage::ExclusiveOpen,
                message: format!("exclusive open {temp_str}: {e}"),
                residue_path: None,
            };
        }
    };

    // 2) 字节拷贝
    let copy_result = copy_bytes(
        req.source.as_mut(),
        sink.as_mut(),
        req.source_total,
        req.control,
        req.progress,
    )
    .await;

    let bytes = match copy_result {
        Ok(bytes) => bytes,
        Err(e) => {
            let cleanup = req.sink.try_remove(&temp_str).await;
            let cleanup_failure = match cleanup {
                Ok(()) => None,
                Err(ProtocolError::TransferCleanupFailed { path, reason }) => {
                    Some(format!("{path}: {reason}"))
                }
                Err(other) => Some(format!("{other}")),
            };
            return StagedOutcome::Failed {
                stage: FailedStage::Copy,
                message: format!("copy: {e}"),
                residue_path: cleanup_failure.is_some().then(|| temp_str.clone()),
            };
        }
    };

    // 3) 取消仲裁
    if req.cancel_decision.load(Ordering::SeqCst) == DECISION_CANCELLED {
        let cleanup = req.sink.try_remove(&temp_str).await;
        let cleanup_failure = match cleanup {
            Ok(()) => None,
            Err(ProtocolError::TransferCleanupFailed { path, reason }) => {
                Some(format!("{path}: {reason}"))
            }
            Err(other) => Some(format!("{other}")),
        };
        return StagedOutcome::Cancelled {
            residue_path: cleanup_failure.is_some().then(|| temp_str.clone()),
            cleanup_failure,
        };
    }

    // 4) 预检重做（下载的 target 在本地，policy 检查同上传）
    let target_existed = req.sink.exists(&target.to_string_lossy()).await;
    if target_existed && !matches!(req.conflict, ConflictPolicy::Overwrite) {
        let cleanup = req.sink.try_remove(&temp_str).await;
        let cleanup_failure = match cleanup {
            Ok(()) => None,
            Err(ProtocolError::TransferCleanupFailed { path, reason }) => {
                Some(format!("{path}: {reason}"))
            }
            Err(other) => Some(format!("{other}")),
        };
        return StagedOutcome::Failed {
            stage: FailedStage::LateConflict,
            message: format!("target appeared before commit: {}", target.display()),
            residue_path: cleanup_failure.is_some().then(|| temp_str.clone()),
        };
    }

    // 5) 提交
    let caps = rshell_protocol::ssh::sftp::SftpCapabilities::default();
    let commit = match req
        .sink
        .safe_commit(&temp_str, &target.to_string_lossy(), caps)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            let cleanup = req.sink.try_remove(&temp_str).await;
            let cleanup_failure = match cleanup {
                Ok(()) => None,
                Err(ProtocolError::TransferCleanupFailed { path, reason }) => {
                    Some(format!("{path}: {reason}"))
                }
                Err(other) => Some(format!("{other}")),
            };
            let residue = if cleanup_failure.is_some() {
                Some(temp_str.clone())
            } else {
                None
            };
            return StagedOutcome::Failed {
                stage: FailedStage::Commit,
                message: format!("commit {}: {e}", target.display()),
                residue_path: residue,
            };
        }
    };

    // 6) 抢占决策点
    match req.cancel_decision.compare_exchange(
        DECISION_PENDING,
        DECISION_COMMITTED,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => StagedOutcome::Completed { bytes, commit },
        Err(DECISION_CANCELLED) => StagedOutcome::Completed { bytes, commit },
        Err(other) => unreachable!("unexpected decision value: {other}"),
    }
}

/// 字节拷贝 —— 借用 [`copy_with_progress`](rshell_protocol::ssh::sftp) 的逻辑。
///
/// 这里不复用 `rshell_protocol::ssh::sftp::copy_with_progress`，因为
/// 它的 `progress` 是泛型闭包而非 `&mut dyn FnMut`，且这里需要把
/// `TransferCancelled` 透传到上层 —— 单独实现一份薄壳避免改动 protocol
/// 接口。
async fn copy_bytes<R, W>(
    reader: &mut R,
    writer: &mut W,
    total: u64,
    control: &mut watch::Receiver<rshell_protocol::ssh::sftp::TransferControl>,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<u64, ProtocolError>
where
    R: AsyncRead + Unpin + ?Sized,
    W: AsyncWrite + Unpin + ?Sized,
{
    use rshell_protocol::ssh::sftp::TransferControl;
    use tokio::io::AsyncWriteExt;

    const CHUNK_SIZE: usize = 64 * 1024;

    progress(0, total);

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut done: u64 = 0;
    loop {
        // 控制信号检查：与 sftp.rs 的 `wait_for_run` 同款语义——先按值 match
        // 归类（Ref 析构后再走 `changed().await`）。
        match *control.borrow_and_update() {
            TransferControl::Cancel => return Err(ProtocolError::TransferCancelled),
            TransferControl::Run => {}
            TransferControl::Pause => {}
        }
        if matches!(*control.borrow(), TransferControl::Pause) {
            if control.changed().await.is_err() {
                return Err(ProtocolError::TransferCancelled);
            }
            continue;
        }

        let n = tokio::io::AsyncReadExt::read(reader, &mut buf)
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

/// R2-T2：让 `TransferTaskInfo` 透出「提交是否原子」。
pub fn commit_strategy_label(strategy: CommitStrategy) -> &'static str {
    match strategy {
        CommitStrategy::PosixRename => "posix_rename",
        CommitStrategy::StandardRename => "standard_rename",
    }
}

#[cfg(test)]
mod tests {
    //! R2-T2：RED 阶段 —— 这些测试**必须**在实现前失败。失败信息：
    //! - `staging_temp_path_str` 与 `staging_local_temp_path`：纯函数，独立可测。
    //! - `run_staged_upload`：覆盖 fresh 目标 / 覆盖 / 晚冲突 / 取消 / commit
    //!   失败 / cleanup 失败 / 独占撞同 target。
    //! - `run_staged_download`：与 upload 对称，覆盖本地 staging。

    use super::*;
    use crate::transfer::test_sink::{FakeRead, FakeSink};
    use rshell_api::types::ConflictPolicy;
    use rshell_protocol::ssh::sftp::TransferControl;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU8;
    use tokio::sync::watch;

    fn run_with<'a>(
        sink: Arc<FakeSink>,
        target: &'a str,
        conflict: ConflictPolicy,
        source: Vec<u8>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = StagedOutcome> + 'a>> {
        let source_total = source.len() as u64;
        let (_tx, mut rx) = watch::channel(TransferControl::Run);
        let temp = staging_temp_path_str(target, "task-uuid");
        let mut progress = |_, _| {};
        let decision = Arc::new(AtomicU8::new(DECISION_PENDING));
        Box::pin(async move {
            run_staged_upload(UploadRequest {
                source: Box::new(FakeRead {
                    bytes: source,
                    pos: 0,
                }),
                source_total,
                target: target.to_string(),
                temp_path: temp,
                conflict,
                sink,
                control: &mut rx,
                progress: &mut progress,
                cancel_decision: decision,
            })
            .await
        })
    }

    #[test]
    fn staging_temp_path_uses_task_uuid_suffix_and_same_directory() {
        assert_eq!(
            staging_temp_path_str("/home/u/a.txt", "task-uuid"),
            "/home/u/a.txt.partial-task-uuid"
        );
        assert_eq!(staging_temp_path_str("/a.txt", "x"), "/a.txt.partial-x");
        // 裸名（无 `/`）兜底：直接拼接
        assert_eq!(staging_temp_path_str("bare.txt", "x"), "bare.txt.partial-x");
    }

    #[test]
    fn staging_local_temp_path_keeps_parent_and_appends_uuid() {
        let dir = std::env::temp_dir();
        let target = dir.join("a.txt");
        let temp = staging_local_temp_path(&target, "task-uuid");
        assert_eq!(temp.parent(), Some(dir.as_path()));
        assert_eq!(
            temp.file_name().and_then(|n| n.to_str()),
            Some("a.txt.partial-task-uuid")
        );
    }

    #[tokio::test]
    async fn upload_to_fresh_target_succeeds_and_cleans_up_temp() {
        // RED 1：fresh 目标 → temp 写完 → commit rename → 无 temp 残留
        let sink = Arc::new(FakeSink::new());
        let payload: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
        let outcome = run_with(
            sink.clone(),
            "/remote/target.bin",
            ConflictPolicy::Fail,
            payload.clone(),
        )
        .await;
        match outcome {
            StagedOutcome::Completed { bytes, .. } => {
                assert_eq!(bytes, payload.len() as u64);
            }
            other => panic!("fresh target 必须完成：{other:?}"),
        }
        // target 已写入；temp 必须不存在
        assert_eq!(sink.snapshot("/remote/target.bin"), Some(payload));
        assert!(sink
            .snapshot("/remote/target.bin.partial-task-uuid")
            .is_none());
        assert_eq!(
            sink.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            sink.cleanup_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "成功路径不应触发 cleanup"
        );
    }

    #[tokio::test]
    async fn upload_with_overwrite_replaces_existing_target() {
        // RED 2：Overwrite policy → 旧 target 被覆盖
        let sink = Arc::new(FakeSink::new().with_target_existing(true));
        // 预填「旧 target」
        {
            let mut files = sink.files.lock().unwrap();
            files.insert("/remote/target.bin", b"OLD".to_vec());
        }
        let payload = b"NEW".to_vec();
        let outcome = run_with(
            sink.clone(),
            "/remote/target.bin",
            ConflictPolicy::Overwrite,
            payload.clone(),
        )
        .await;
        match outcome {
            StagedOutcome::Completed { .. } => {}
            other => panic!("Overwrite 必须完成：{other:?}"),
        }
        assert_eq!(sink.snapshot("/remote/target.bin"), Some(payload));
    }

    #[tokio::test]
    async fn upload_late_conflict_fails_with_no_target_change() {
        // RED 3：预检时不存在 + Overwrite 但提交前 target 出现：必须失败 + 旧 target 保留
        let sink = Arc::new(FakeSink::new().with_target_existing(true));
        // 预填「已有 target」
        {
            let mut files = sink.files.lock().unwrap();
            files.insert("/remote/target.bin", b"OLD".to_vec());
        }
        let outcome = run_with(
            sink.clone(),
            "/remote/target.bin",
            ConflictPolicy::Fail,
            b"NEW".to_vec(),
        )
        .await;
        match outcome {
            StagedOutcome::Failed {
                stage,
                residue_path,
                ..
            } => {
                assert_eq!(stage, FailedStage::LateConflict);
                // cleanup 默认成功 → 没有残留
                assert!(residue_path.is_none());
            }
            other => panic!("晚冲突必须报 Failed：{other:?}"),
        }
        assert_eq!(
            sink.snapshot("/remote/target.bin"),
            Some(b"OLD".to_vec()),
            "晚冲突时旧 target 必须保留"
        );
        assert_eq!(
            sink.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn upload_cancel_before_commit_preserves_target_and_cleans_temp() {
        // RED 4：取消在拷贝完成后 / commit 前到达 —— target 保留 + temp 清理
        let sink = Arc::new(FakeSink::new());
        // 预填「目标不存在」的语义，并预先放一个 target 让 cancel 后断言它没变
        {
            let mut files = sink.files.lock().unwrap();
            files.insert("/remote/target.bin", b"OLD".to_vec());
        }
        let (_tx, mut rx) = watch::channel(TransferControl::Run);
        let temp = staging_temp_path_str("/remote/target.bin", "task-uuid");
        let mut progress = |_, _| {};
        let decision = Arc::new(AtomicU8::new(DECISION_PENDING));

        // 拷贝完成后立刻设置取消标志——模拟「拷贝进行中、commit 即将发起时取消到达」
        decision.store(DECISION_CANCELLED, std::sync::atomic::Ordering::SeqCst);

        let outcome = run_staged_upload(UploadRequest {
            source: Box::new(FakeRead {
                bytes: b"NEW".to_vec(),
                pos: 0,
            }),
            source_total: 3,
            target: "/remote/target.bin".to_string(),
            temp_path: temp.clone(),
            conflict: ConflictPolicy::Overwrite,
            sink: sink.clone(),
            control: &mut rx,
            progress: &mut progress,
            cancel_decision: decision,
        })
        .await;

        match outcome {
            StagedOutcome::Cancelled {
                residue_path,
                cleanup_failure,
            } => {
                assert!(residue_path.is_none(), "清理成功时不报 residue");
                assert!(cleanup_failure.is_none());
            }
            other => panic!("取消必须报 Cancelled：{other:?}"),
        }
        // target 未变（旧内容保留）
        assert_eq!(sink.snapshot("/remote/target.bin"), Some(b"OLD".to_vec()));
        // temp 必须清理
        assert!(sink.snapshot(&temp).is_none());
        // commit 必须没发生
        assert_eq!(
            sink.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn upload_commit_failure_preserves_target_and_reports_cleanup_status() {
        // RED 5：commit 失败 → target 保留 + temp 已被清理（success）或报告 residue（failure）
        let sink = Arc::new(FakeSink::new().with_commit_failing());
        let outcome = run_with(
            sink.clone(),
            "/remote/target.bin",
            ConflictPolicy::Overwrite,
            b"payload".to_vec(),
        )
        .await;
        match outcome {
            StagedOutcome::Failed {
                stage,
                residue_path,
                ..
            } => {
                assert_eq!(stage, FailedStage::Commit);
                // cleanup 默认成功 → 没有残留，residue_path 为 None
                assert!(
                    residue_path.is_none(),
                    "cleanup 成功时不应报 residue（实际无残留）"
                );
            }
            other => panic!("commit 失败必须报 Failed：{other:?}"),
        }
        // target 必须保留——失败时不应被改写
        assert_eq!(
            sink.snapshot("/remote/target.bin"),
            None,
            "commit 失败不应改写 target"
        );
        assert_eq!(
            sink.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn upload_commit_failure_and_cleanup_failure_both_reported() {
        // RED 6：commit + cleanup 都失败 → residue 报告原始失败 + cleanup 失败
        let sink = Arc::new(FakeSink::new().with_commit_failing().with_cleanup_failing());
        let outcome = run_with(
            sink.clone(),
            "/remote/target.bin",
            ConflictPolicy::Overwrite,
            b"payload".to_vec(),
        )
        .await;
        match outcome {
            StagedOutcome::Failed {
                stage,
                residue_path,
                message,
            } => {
                assert_eq!(stage, FailedStage::Commit);
                assert!(residue_path.is_some());
                assert!(
                    message.contains("commit"),
                    "原始 commit 失败必须保留：{message}"
                );
            }
            other => panic!("commit + cleanup 双失败必须报 Failed：{other:?}"),
        }
        // temp 仍存在 —— cleanup 失败
        assert!(sink
            .snapshot("/remote/target.bin.partial-task-uuid")
            .is_some());
    }

    #[tokio::test]
    async fn upload_exclusive_collision_on_same_target_fails() {
        // RED 7：第二次 exclusive_open 失败（两个并发任务撞同一 target）
        let sink = Arc::new(FakeSink::new());
        // 手工预填 temp 模拟另一个任务已经抢到
        let temp = staging_temp_path_str("/remote/target.bin", "task-uuid");
        {
            let mut files = sink.files.lock().unwrap();
            files.insert(&temp, b"OCCUPIED".to_vec());
        }
        let outcome = run_with(
            sink.clone(),
            "/remote/target.bin",
            ConflictPolicy::Overwrite,
            b"NEW".to_vec(),
        )
        .await;
        match outcome {
            StagedOutcome::Failed {
                stage,
                residue_path,
                ..
            } => {
                assert_eq!(stage, FailedStage::ExclusiveOpen);
                assert!(residue_path.is_none());
            }
            other => panic!("独占撞同 target 必须报 Failed：{other:?}"),
        }
        // 旧 temp 未被改写
        assert_eq!(sink.snapshot(&temp), Some(b"OCCUPIED".to_vec()));
        assert_eq!(
            sink.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn upload_cancel_during_commit_is_completed_with_target_replaced() {
        // RED 8：commit 已在飞 → 取消到达 → commit 成功 → 任务 Completed
        // （「取消任务却替换目标」的反例，证明：一旦 commit 成功，取消意图被吞）
        let decision = Arc::new(AtomicU8::new(DECISION_PENDING));
        let decision_for_hook = decision.clone();
        let sink = Arc::new(FakeSink::new().with_on_commit(move |_from, _to| {
            // safe_commit 进入时立刻把决策置为 CANCELLED —— 模拟「commit 期间取消到达」
            decision_for_hook.store(DECISION_CANCELLED, std::sync::atomic::Ordering::SeqCst);
        }));
        let (_tx, mut rx) = watch::channel(TransferControl::Run);
        let temp = staging_temp_path_str("/remote/target.bin", "task-uuid");
        let mut progress = |_, _| {};

        let outcome = run_staged_upload(UploadRequest {
            source: Box::new(FakeRead {
                bytes: b"NEW".to_vec(),
                pos: 0,
            }),
            source_total: 3,
            target: "/remote/target.bin".to_string(),
            temp_path: temp,
            conflict: ConflictPolicy::Overwrite,
            sink: sink.clone(),
            control: &mut rx,
            progress: &mut progress,
            cancel_decision: decision,
        })
        .await;

        // 关键：commit CAS 失败但函数仍返回 Completed——因为 commit 已成功
        match outcome {
            StagedOutcome::Completed { bytes, .. } => assert_eq!(bytes, 3),
            other => panic!("commit 成功后取消到达 → Completed：{other:?}"),
        }
        assert_eq!(
            sink.snapshot("/remote/target.bin"),
            Some(b"NEW".to_vec()),
            "commit 已生效：target 必须被替换"
        );
        assert_eq!(
            sink.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn download_to_fresh_local_target_succeeds_and_cleans_up_temp() {
        // RED 9：下载 —— source (远端) → 本地 temp → 本地 commit → 无残留
        let sink = Arc::new(FakeSink::new());
        let payload: Vec<u8> = (0..512u32).map(|i| (i % 251) as u8).collect();
        // 预填「远端源」
        {
            let mut files = sink.files.lock().unwrap();
            files.insert("/remote/source.bin", payload.clone());
        }
        let (_tx, mut rx) = watch::channel(TransferControl::Run);
        let mut progress = |_, _| {};
        let decision = Arc::new(AtomicU8::new(DECISION_PENDING));
        let target = PathBuf::from("/local/target.bin");
        let temp = staging_local_temp_path(&target, "task-uuid");

        // 先开 source reader，再传给 run_staged_download
        let source_reader = sink
            .open_read("/remote/source.bin")
            .await
            .expect("预填 source 后可读");

        let outcome = run_staged_download(DownloadRequest {
            source: source_reader,
            source_total: payload.len() as u64,
            target: target.clone(),
            temp_path: temp.clone(),
            conflict: ConflictPolicy::Fail,
            sink: sink.clone(),
            control: &mut rx,
            progress: &mut progress,
            cancel_decision: decision,
        })
        .await;

        match outcome {
            StagedOutcome::Completed { bytes, .. } => assert_eq!(bytes, payload.len() as u64),
            other => panic!("fresh 本地目标必须完成：{other:?}"),
        }
        assert_eq!(sink.snapshot(&target.to_string_lossy()), Some(payload));
        assert!(sink.snapshot(&temp.to_string_lossy()).is_none());
    }

    #[tokio::test]
    async fn download_cancel_before_commit_preserves_local_target() {
        // RED 10：下载版取消仲裁
        let sink = Arc::new(FakeSink::new());
        let payload: Vec<u8> = b"remote-source".to_vec();
        {
            let mut files = sink.files.lock().unwrap();
            files.insert("/remote/source.bin", payload.clone());
            files.insert("/local/target.bin", b"OLD-LOCAL".to_vec());
        }
        let (_tx, mut rx) = watch::channel(TransferControl::Run);
        let mut progress = |_, _| {};
        let decision = Arc::new(AtomicU8::new(DECISION_PENDING));
        decision.store(DECISION_CANCELLED, std::sync::atomic::Ordering::SeqCst);

        let target = PathBuf::from("/local/target.bin");
        let temp = staging_local_temp_path(&target, "task-uuid");

        let source_reader = sink
            .open_read("/remote/source.bin")
            .await
            .expect("source 可读");

        let outcome = run_staged_download(DownloadRequest {
            source: source_reader,
            source_total: payload.len() as u64,
            target: target.clone(),
            temp_path: temp.clone(),
            conflict: ConflictPolicy::Overwrite,
            sink: sink.clone(),
            control: &mut rx,
            progress: &mut progress,
            cancel_decision: decision,
        })
        .await;

        match outcome {
            StagedOutcome::Cancelled { residue_path, .. } => {
                assert!(residue_path.is_none(), "cleanup 成功时不报 residue");
            }
            other => panic!("下载取消必须报 Cancelled：{other:?}"),
        }
        assert_eq!(
            sink.snapshot(&target.to_string_lossy()),
            Some(b"OLD-LOCAL".to_vec()),
            "取消时旧本地 target 必须保留"
        );
        assert!(sink.snapshot(&temp.to_string_lossy()).is_none());
    }

    #[tokio::test]
    async fn download_commit_failure_preserves_local_target() {
        // RED 11：下载版 commit 失败
        let sink = Arc::new(FakeSink::new().with_commit_failing());
        let payload: Vec<u8> = b"remote".to_vec();
        {
            let mut files = sink.files.lock().unwrap();
            files.insert("/remote/source.bin", payload.clone());
            files.insert("/local/target.bin", b"OLD".to_vec());
        }
        let (_tx, mut rx) = watch::channel(TransferControl::Run);
        let mut progress = |_, _| {};
        let decision = Arc::new(AtomicU8::new(DECISION_PENDING));
        let target = PathBuf::from("/local/target.bin");
        let temp = staging_local_temp_path(&target, "task-uuid");

        let source_reader = sink
            .open_read("/remote/source.bin")
            .await
            .expect("source 可读");

        let outcome = run_staged_download(DownloadRequest {
            source: source_reader,
            source_total: payload.len() as u64,
            target: target.clone(),
            temp_path: temp.clone(),
            conflict: ConflictPolicy::Overwrite,
            sink: sink.clone(),
            control: &mut rx,
            progress: &mut progress,
            cancel_decision: decision,
        })
        .await;

        match outcome {
            StagedOutcome::Failed { stage, .. } => assert_eq!(stage, FailedStage::Commit),
            other => panic!("下载 commit 失败必须报 Failed：{other:?}"),
        }
        assert_eq!(
            sink.snapshot(&target.to_string_lossy()),
            Some(b"OLD".to_vec()),
            "commit 失败时旧本地 target 必须保留"
        );
    }
}
