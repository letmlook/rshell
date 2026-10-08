//! SFTP 文件传输客户端
//!
//! 基于 russh-sftp 实现 SFTP 文件传输功能：
//! - 远程目录浏览
//! - 文件上传/下载
//! - 文件元数据查询

use crate::ProtocolError;
use rshell_api::types::{FilePermissions, FileType, RemoteFileEntry};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;
use tracing::{debug, info};

/// 单次读写分块大小（64 KiB）
const CHUNK_SIZE: usize = 64 * 1024;

/// 下载方向的在途 READ 并发度。
///
/// SFTP 的 READ 是请求/响应协议：russh-sftp 的 `File::poll_read` 只有一个
/// `f_read` 槽（client/fs/file.rs），一次只发一个 READ 并等待响应，因此串行
/// 读取的吞吐上限是 `CHUNK_SIZE / RTT`——局域网 RTT 再低也会被这一个往返卡住。
/// 上传方向相反，`poll_write` 用 `write_nowait` 支持 `max_concurrent_writes`
/// 路并发，所以上传明显快于下载。
///
/// 这里把下载改成多个在途 READ 并发，与上传的并发度对齐。
const DOWNLOAD_CONCURRENCY: usize = 8;

/// 下载并发度低于该值时退回串行拷贝：省掉多句柄/多任务的固定开销。
const DOWNLOAD_PIPELINE_MIN_BYTES: u64 = (CHUNK_SIZE * 2) as u64;

/// 通用句柄池：把「借出/归还」与具体 I/O 解耦，便于脱离 SFTP 服务器单测。
///
/// 容量上限保证池不会因为归还次数多于借出次数而无限增长（借出后任务被中止、
/// 又走了一次兜底新建时会出现归还多于借出）。
struct HandlePool<T> {
    idle: tokio::sync::Mutex<Vec<T>>,
    capacity: usize,
}

impl<T> HandlePool<T> {
    fn with_handles(handles: Vec<T>) -> Self {
        let capacity = handles.len();
        Self {
            idle: tokio::sync::Mutex::new(handles),
            capacity,
        }
    }

    /// 借一个句柄；池空返回 None，由调用方决定是兜底新建还是报错
    async fn acquire(&self) -> Option<T> {
        self.idle.lock().await.pop()
    }

    /// 归还句柄；超出容量时直接丢弃（句柄只是缓存，Drop 会关闭它）
    async fn release(&self, handle: T) {
        let mut idle = self.idle.lock().await;
        if idle.len() < self.capacity {
            idle.push(handle);
        }
    }

    /// 仅测试用：断言池内空闲句柄数，生产路径不读这个计数
    #[cfg(test)]
    async fn idle_count(&self) -> usize {
        self.idle.lock().await.len()
    }
}
/// 远端文件句柄池：并发区间读取共用固定数量的已打开句柄。
///
/// 逐区间 `session.open()` 会把 OPEN/CLOSE 放大到「每 64 KiB 一对」——1 GiB
/// 文件就是 16384 次 OPEN + 16384 次 CLOSE，每次一个往返，协议消息数是数据的
/// 3 倍，远端还要反复建/销句柄。池化后整个传输只开 `capacity` 次。
///
/// 池空时（并发数被调高，或某个任务被中止时连同句柄一起丢弃）退回现开一个，
/// 保证读取不会因为句柄泄漏而永久阻塞。
#[derive(Clone)]
struct RemoteHandlePool {
    session: Arc<russh_sftp::client::SftpSession>,
    path: Arc<str>,
    handles: Arc<HandlePool<russh_sftp::client::fs::File>>,
}

impl RemoteHandlePool {
    async fn open(
        session: Arc<russh_sftp::client::SftpSession>,
        path: &str,
        capacity: usize,
    ) -> Result<Self, ProtocolError> {
        let mut handles = Vec::with_capacity(capacity);
        for _ in 0..capacity.max(1) {
            handles.push(session.open(path).await.map_err(|e| {
                ProtocolError::ProtocolError(format!("Failed to open remote file: {e}"))
            })?);
        }
        Ok(Self {
            session,
            path: Arc::from(path),
            handles: Arc::new(HandlePool::with_handles(handles)),
        })
    }

    /// 借一个句柄读指定区间，读完（无论成功失败）归还。
    async fn read_range(&self, offset: u64, len: usize) -> Result<Vec<u8>, ProtocolError> {
        let mut file = match self.handles.acquire().await {
            Some(file) => file,
            None => self.session.open(self.path.as_ref()).await.map_err(|e| {
                ProtocolError::ProtocolError(format!(
                    "Failed to open remote file for range at {offset}: {e}"
                ))
            })?,
        };

        let result = async {
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| {
                    ProtocolError::ProtocolError(format!(
                        "Failed to seek remote file to {offset}: {e}"
                    ))
                })?;
            let mut buf = vec![0u8; len];
            file.read_exact(&mut buf).await.map_err(|e| {
                ProtocolError::ProtocolError(format!("range read failed at {offset} (+{len}): {e}"))
            })?;
            Ok(buf)
        }
        .await;

        self.handles.release(file).await;
        result
    }
}

/// 传输控制信号
///
/// 由上层传输服务通过 `watch` 通道按 task_id 下发，cancel 与 pause 共用
/// 同一通道；拷贝循环在每个分块前检查：暂停时挂起等待恢复，取消时立即中止。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferControl {
    /// 继续传输
    Run,
    /// 暂停（在下一分块前挂起，恢复后从已传字节继续）
    Pause,
    /// 取消（在下一分块前中止，已写入的部分保留）
    Cancel,
}

/// R2-T2：服务器声明的 SFTP 扩展集合，用于决定安全提交策略。
///
/// 仅记录**本轮需要用到**的扩展——其它扩展（`statvfs`、`expand-path`、
/// `hardlink`、`fsync`、`limits`）由 `russh-sftp` 高层 API 自行处理。
/// 这里聚焦「提交语义」相关的两条：`posix-rename@openssh.com` 与
/// `hardlink@openssh.com`（后者仅作探测参考，提交路径不会用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SftpCapabilities {
    /// `posix-rename@openssh.com`：远端接受 POSIX 语义的重命名（原子替换）。
    pub posix_rename: bool,
}

/// R2-T2：传输状态机用的 sink 抽象 —— 把远端（`SftpClient`）和本地
/// (`tokio::fs`) 统一成同一组操作：独占打开、stat、safe_commit、try_remove。
///
/// 该 trait 的存在意义是把「提交语义」与具体文件系统解耦：
/// - `RemoteSink`（rshell-protocol 内）：把 `SftpClient` 的方法拼成
///   staging 操作；
/// - `LocalSink`（rshell-protocol 内）：把 `tokio::fs` 拼成同样的接口；
/// - 测试里：用一个内存 fake 实现来驱动状态机分支
///   （取消/提交竞态、晚冲突、清理失败等），不必起真实服务器。
///
/// `commit` 与 `cleanup` 都返回 `Result`，失败一律透传：上层（core
/// 传输服务）按错误种类写终态与 residue；trait 不吞错。
///
/// 用 `async_trait` 而不是原生 AFIT 是因为 `&dyn TransferSink` 要走
/// 动态分发——`impl Trait` 不能在 `dyn` 里使用，`async_trait` 展开后
/// 的 `Box<dyn Future + Send>` 才能满足这个需求。
#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
pub trait TransferSink: Send + Sync {
    /// 远端 staging temp 路径的「存在性」检测（提交前再做一次）。
    /// 用 `exists()` 而非 `metadata()`：stat 失败一律视为「存在」，
    /// 与现有 `SftpClient::exists` 语义一致，宁可弹冲突也不静默覆盖。
    async fn exists(&self, path: &str) -> bool;

    /// 独占方式打开/创建文件用于 staging；返回 `AsyncWrite + Send`。
    async fn exclusive_open_write(
        &self,
        path: &str,
    ) -> Result<Box<dyn tokio::io::AsyncWrite + Unpin + Send>, ProtocolError>;

    /// 打开已存在的目标文件用于下载读。
    async fn open_read(
        &self,
        path: &str,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Unpin + Send>, ProtocolError>;

    /// 安全提交（rename）；成功即目标被替换，失败保留旧目标。
    async fn safe_commit(
        &self,
        from: &str,
        to: &str,
        caps: SftpCapabilities,
    ) -> Result<CommitOutcome, ProtocolError>;

    /// 尽力删除临时文件；「不存在」视为成功。
    async fn try_remove(&self, path: &str) -> Result<(), ProtocolError>;

    /// 当前 sink 是远程还是本地（仅供日志与 residue 报告区分）。
    fn is_remote(&self) -> bool;
}

/// R2-T2：提交阶段使用的策略，由 `SftpCapabilities` 决定。
///
/// 语义差别：
/// - `PosixRename`：明确走 `posix-rename@openssh.com` 扩展，远端保证
///   **原子替换**——若 `newpath` 已存在则覆盖，旧 `newpath`（若有）消失。
///   整个过程对其它客户端而言是原子的。
/// - `StandardRename`：仅走标准 `SSH_FXP_RENAME`（draft-02）。RFC 文本里
///   对「`newpath` 已存在时怎么处理」并未强制；多数主流服务器在实践中
///   是替换，但**不能**作为通用保证。我们只声明「尽量原子替换」，不
///   承诺。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStrategy {
    /// 远端声明 `posix-rename@openssh.com`：原子替换有保证
    PosixRename,
    /// 走标准 SSH_FXP_RENAME：不普遍承诺原子性，但成功即可生效
    StandardRename,
}

/// R2-T2：安全提交的执行结果——附带选用的策略，供上层如实报告。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    pub strategy: CommitStrategy,
}

/// 由「目标路径 + 任务 UUID」拼出 staging temp 路径。
///
/// 选择与目标同目录、同名后追加 `.partial-<task_uuid>`：
/// - 同目录：保证最终 rename 在大多数文件系统上**不需要跨目录**，
///   跨目录 rename 在某些远端 / 文件系统上可能不是原子替换。
/// - `.partial-<uuid>`：后缀可识别（前端/扫描器能定位「属于某次任务的」残留），
///   `task_uuid` 区分并发任务，避免两个任务抢同一个临时路径。
pub fn staging_temp_path(target: &str, task_uuid: &str) -> String {
    if let Some(slash) = target.rfind('/') {
        let dir = if slash == 0 { "/" } else { &target[..slash] };
        let name = &target[slash + 1..];
        format!("{dir}/{name}.partial-{task_uuid}")
    } else {
        // 远端路径通常以 `/` 开头；这里兜底裸名
        format!("{target}.partial-{task_uuid}")
    }
}

/// SFTP 客户端
///
/// 封装 russh_sftp::client::SftpSession，提供高层文件操作接口。
///
/// `SftpSession` 自身不是 `Clone`，但全部方法都取 `&self`；用 `Arc` 共享，
/// 让并发区间下载的多个任务能同时在同一条 SFTP 会话上发 READ
/// （russh-sftp 按请求 id 多路复用，并发请求是协议层支持的）。
pub struct SftpClient {
    session: Arc<russh_sftp::client::SftpSession>,
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
        Ok(Self {
            session: Arc::new(session),
        })
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
    ///
    /// `control` 是 pause/resume/cancel 共用的控制信号：每个分块前检查一次，
    /// 暂停时挂起等待恢复，取消时返回 `ProtocolError::TransferCancelled`。
    pub async fn upload<F>(
        &self,
        local: &PathBuf,
        remote: &str,
        conflict_overwrite: bool,
        control: &mut watch::Receiver<TransferControl>,
        mut progress: F,
    ) -> Result<u64, ProtocolError>
    where
        F: FnMut(u64, u64),
    {
        info!(local = %local.display(), remote = %remote, "Uploading file");

        // 覆盖策略：先探测再 create。若在 create 之后才判断，Truncate 标志
        // 已经把既有文件清空了——那时报冲突也来不及。
        if !conflict_overwrite && self.exists(remote).await {
            return Err(ProtocolError::TransferConflict(remote.to_string()));
        }

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

        let copied = copy_with_progress(
            &mut source,
            &mut target,
            total,
            CHUNK_SIZE,
            control,
            &mut progress,
        )
        .await?;
        drop(target);

        info!(remote = %remote, bytes = copied, "Upload completed");
        Ok(copied)
    }

    /// 下载远程文件到本地（分块写入，边传边回调进度）
    ///
    /// `control` 的语义与 [`Self::upload`] 相同。
    ///
    /// 大文件走并发区间读取（见 [`copy_pipelined`]）：每个区间用独立句柄
    /// `seek` 到区间起点后 `read_exact`，最多 [`DOWNLOAD_CONCURRENCY`] 个
    /// READ 同时在途；写入侧仍严格按 offset 升序串行，落盘内容与串行版本一致。
    /// 小文件退回串行，避免多句柄的固定开销。
    pub async fn download<F>(
        &self,
        remote: &str,
        local: &PathBuf,
        control: &mut watch::Receiver<TransferControl>,
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

        // 确保本地目录存在
        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                ProtocolError::ProtocolError(format!("Failed to create local dir: {}", e))
            })?;
        }
        let mut target = tokio::fs::File::create(local).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to create local file: {}", e))
        })?;

        let copied = if total >= DOWNLOAD_PIPELINE_MIN_BYTES {
            // 句柄池：并发区间共用固定数量的远端句柄。
            //
            // 每个区间各开一个句柄看似无害，实则把 OPEN/CLOSE 放大到「每 64 KiB
            // 一对」——1 GiB 文件就是 16384 次 OPEN + 16384 次 CLOSE，每次一个
            // 往返，协议消息数是数据的 3 倍，远端还要反复建/销句柄。池化后整个
            // 传输只开 DOWNLOAD_CONCURRENCY 次。
            let pool = RemoteHandlePool::open(
                self.session.clone(),
                remote,
                DOWNLOAD_CONCURRENCY.min(plan_ranges(total, CHUNK_SIZE).len()),
            )
            .await?;
            copy_pipelined(
                total,
                move |offset, len| {
                    let pool = pool.clone();
                    async move { pool.read_range(offset, len).await }
                },
                &mut target,
                control,
                &mut progress,
            )
            .await?
        } else {
            let mut source = self.session.open(remote).await.map_err(|e| {
                ProtocolError::ProtocolError(format!("Failed to open remote file: {}", e))
            })?;
            copy_with_progress(
                &mut source,
                &mut target,
                total,
                CHUNK_SIZE,
                control,
                &mut progress,
            )
            .await?
        };

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

    /// 远端用户的工作目录（登录后默认所在目录）
    ///
    /// 用 SFTP `realpath(".")` 解析：sftp 子系统启动时的当前目录即为登录用户
    /// 的 home，远端无需额外支持 `~` 展开。解析失败时回退到 `/`，
    /// 保证调用方总有可用目录（与旧行为一致，不把失败暴露为空白面板）。
    pub async fn home_dir(&self) -> Result<String, ProtocolError> {
        match self.session.canonicalize(".").await {
            Ok(dir) if !dir.trim().is_empty() => Ok(dir),
            Ok(_) | Err(_) => Ok("/".to_string()),
        }
    }

    /// 远端路径是否已存在。用于传输前的覆盖冲突判定。
    ///
    /// 只区分「存在 / 不存在」：stat 的其他失败（权限、无权限进入父目录等）
    /// 一律当作**存在**，宁可让用户在对话框里看到「已存在」也不静默覆盖。
    pub async fn exists(&self, path: &str) -> bool {
        self.session.metadata(path).await.is_ok()
    }

    /// R2-T2：探测远端 SFTP 扩展，仅返回本轮提交语义相关的两条。
    ///
    /// `russh-sftp 2.4` 的 `SftpSession` 把 `RawSftpSession` 藏在私有字段里——
    /// 没有公开 `extended()` 入口，所以本轮**没**法直接探测
    /// `posix-rename@openssh.com`。详见研究文档 capability 调查。
    ///
    /// 保守策略：所有字段返回 `false`，提交路径走标准 `SSH_FXP_RENAME`；
    /// 一旦后续能访问 raw handle，这里就能升级为真正的探测。
    /// 此函数保留是为了**契约**：上层按「探测→决策→提交」三步走，
    /// 未来升级时不改上层调用方。
    pub async fn probe_capabilities(&self) -> Result<SftpCapabilities, ProtocolError> {
        Ok(SftpCapabilities::default())
    }

    /// R2-T2：以「独占」方式打开/创建临时文件用于 staging。
    ///
    /// `OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE` 等价于
    /// POSIX 的 `O_CREAT | O_EXCL`：文件已存在时**拒绝**创建，避免覆盖另一个
    /// 任务的临时文件（两个任务撞同一目标路径就会在这里失败）。
    pub async fn open_exclusive(
        &self,
        path: &str,
    ) -> Result<russh_sftp::client::fs::File, ProtocolError> {
        use russh_sftp::protocol::OpenFlags;
        let handle = self
            .session
            .open_with_flags(
                path,
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .map_err(|e| {
                ProtocolError::ProtocolError(format!("Failed to open exclusive file: {e}"))
            })?;
        Ok(handle)
    }

    /// R2-T2：把 `SftpClient` 包成 `TransferSink`，供 core 传输服务使用。
    ///
    /// 生命周期与 `SftpClient` 相同：`SshClientHandle` 关闭时 SFTP 通道一并关闭，
    /// 这里仅持有 `Arc<SftpClient>`，**不**额外拉长 SFTP 会话寿命。
    pub fn as_sink(&self) -> RemoteSink {
        RemoteSink {
            client: Arc::new(self.clone_for_sink()),
        }
    }

    /// 给 `as_sink` 用：`SftpClient` 内部仅 `Arc<SftpSession>`，克隆代价低。
    fn clone_for_sink(&self) -> SftpClient {
        SftpClient {
            session: self.session.clone(),
        }
    }

    /// R2-T2：把临时文件提交到目标位置（rename），附带原子性信息。
    ///
    /// 当前实现走标准 `SSH_FXP_RENAME`：OpenSSH 等主流服务器在「`newpath`
    /// 已存在时」会替换（RFC 文本未强制），但**不**作为通用保证返回。
    /// `CommitStrategy::StandardRename` 的语义就是「成功即生效，不普遍承诺
    /// 原子性」——上游据实报告，由 `TransferTaskInfo` 透出。
    ///
    /// 何时能升级到 `posix-rename@openssh.com`：见研究文档 capability 调查。
    /// 调用前**必须**再次检查目标是否存在——本客户端把策略决定提前到
    /// 此处之外的位置，避免「先删后改」的脏路径（ADR-0002 明令禁止）。
    pub async fn safe_commit(
        &self,
        from: &str,
        to: &str,
        _capabilities: SftpCapabilities,
    ) -> Result<CommitOutcome, ProtocolError> {
        self.session
            .rename(from, to)
            .await
            .map_err(|e| ProtocolError::TransferCommitFailed {
                path: to.to_string(),
                reason: format!("rename failed: {e}"),
            })?;
        Ok(CommitOutcome {
            strategy: CommitStrategy::StandardRename,
        })
    }

    /// R2-T2：尽力删除临时文件，吞掉「不存在」类错误。
    ///
    /// 调用方在 cancel / commit-failure 之后做清理：远端可能已被网络断开，
    /// 此时应当返回失败而不是 panic——上层把错误传回终端状态里报告。
    pub async fn try_remove(&self, path: &str) -> Result<(), ProtocolError> {
        match self.session.remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("No such file") || msg.contains("not found") {
                    Ok(())
                } else {
                    Err(ProtocolError::TransferCleanupFailed {
                        path: path.to_string(),
                        reason: msg,
                    })
                }
            }
        }
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

/// R2-T2：`TransferSink` 的远端实现，把 SFTP 操作拼成 staging 接口。
///
/// 整个类型仅一个 `Arc<SftpClient>`，方法全部委托给它——上层按 trait
/// 注入，可以在测试里换成内存 fake。
#[derive(Clone)]
pub struct RemoteSink {
    client: Arc<SftpClient>,
}

#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
impl TransferSink for RemoteSink {
    async fn exists(&self, path: &str) -> bool {
        self.client.exists(path).await
    }

    async fn exclusive_open_write(
        &self,
        path: &str,
    ) -> Result<Box<dyn tokio::io::AsyncWrite + Unpin + Send>, ProtocolError> {
        let file = self.client.open_exclusive(path).await?;
        Ok(Box::new(file))
    }

    async fn open_read(
        &self,
        path: &str,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Unpin + Send>, ProtocolError> {
        let file = self.client.session.open(path).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to open remote file: {e}"))
        })?;
        Ok(Box::new(file))
    }

    async fn safe_commit(
        &self,
        from: &str,
        to: &str,
        caps: SftpCapabilities,
    ) -> Result<CommitOutcome, ProtocolError> {
        self.client.safe_commit(from, to, caps).await
    }

    async fn try_remove(&self, path: &str) -> Result<(), ProtocolError> {
        self.client.try_remove(path).await
    }

    fn is_remote(&self) -> bool {
        true
    }
}

/// 分块拷贝，进度变化时回调 `(bytes_done, total)`。
///
/// 开始前先回调一次 `(0, total)`，返回前保证发出终帧；
/// 声明的 `total` 与实际拷贝字节数不一致（源在传输期间被改写）时报错，
/// 避免远端留下被截断的文件。
/// 每个分块前检查 `control`：暂停时在分块间挂起（字节停止增长，恢复后
/// 从已传字节继续），取消时立即返回 `TransferCancelled`。
async fn copy_with_progress<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    total: u64,
    chunk_size: usize,
    control: &mut watch::Receiver<TransferControl>,
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
        wait_for_run(control).await?;
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

/// 把 `total` 字节切成连续不重叠的区间 `(offset, len)`，按 offset 升序。
///
/// 纯函数：分块正确性直接决定落盘内容是否损坏，必须独立可测。
/// `total == 0` 返回空区间；`chunk == 0` 视为 1，避免死循环。
fn plan_ranges(total: u64, chunk: usize) -> Vec<(u64, usize)> {
    let step = chunk.max(1) as u64;
    let mut ranges = Vec::with_capacity(total.div_ceil(step) as usize);
    let mut offset = 0u64;
    while offset < total {
        let len = step.min(total - offset);
        ranges.push((offset, len as usize));
        offset += len;
    }
    ranges
}

/// 在途区间任务的 RAII 中止守卫（R3-08）。
///
/// 丢弃 `VecDeque<JoinHandle>` 并**不会**中止任务：`JoinHandle` 被 drop 时
/// 任务转为游离态继续在 runtime 上跑，于是最多 `DOWNLOAD_CONCURRENCY` 个
/// 脱离管理的任务会继续发 64 KiB 的 SFTP READ，占住池化句柄与远端文件句柄，
/// 和后续传输抢同一批槽位。
///
/// 因此清理必须绑定在「离开作用域」上，而不是散落在各个错误分支里：
/// `copy_pipelined` 里任何提前返回（含 `wait_for_run` 的 `?` 取消检查、
/// 本地写失败的 `?`）都会触发 `Drop`，在途任务一定被 abort。
struct AbortOnDrop<T> {
    handles: VecDeque<tokio::task::JoinHandle<T>>,
}

impl<T> AbortOnDrop<T> {
    fn new() -> Self {
        Self {
            handles: VecDeque::new(),
        }
    }

    fn push_back(&mut self, handle: tokio::task::JoinHandle<T>) {
        self.handles.push_back(handle);
    }

    /// 取队首（最早派发 = offset 最小），保证按序落盘。
    fn pop_front(&mut self) -> Option<tokio::task::JoinHandle<T>> {
        self.handles.pop_front()
    }

    fn len(&self) -> usize {
        self.handles.len()
    }

    fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// 取出并 abort 全部在途任务。
    fn abort_all(&mut self) {
        for handle in self.handles.drain(..) {
            handle.abort();
        }
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.abort_all();
    }
}

/// 按区间顺序（offset 升序）并发生成区间内容，再**严格按序**写给 `writer`。
///
/// `fetch(offset, len)` 由调用方提供，负责发出单个区间的远端读取，返回恰好
/// `len` 字节。并发只发生在读取侧；写入侧按序串行，因此：
/// - 落盘内容与区间划分一致，与完成顺序无关；
/// - `done` 单调递增，进度回调语义与串行版本一致。
///
/// 暂停/取消在派发新区间前和每次写入前各检查一次，与串行版本同款语义。
async fn copy_pipelined<F, Fut, W, P>(
    total: u64,
    fetch: F,
    writer: &mut W,
    control: &mut watch::Receiver<TransferControl>,
    progress: &mut P,
) -> Result<u64, ProtocolError>
where
    F: Fn(u64, usize) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, ProtocolError>> + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin,
    P: FnMut(u64, u64),
{
    let ranges = plan_ranges(total, CHUNK_SIZE);
    progress(0, total);

    let mut done: u64 = 0;
    // R3-08：在途任务由守卫托管，任何提前返回都会在 `Drop` 里 abort 掉它们
    // （原来只在三个错误分支上显式清理，取消检查与本地写失败的 `?` 会漏掉）。
    let mut pending = AbortOnDrop::<Result<Vec<u8>, ProtocolError>>::new();
    let mut next = 0usize;

    loop {
        // 派发阶段：把在途数补到并发上限。
        while next < ranges.len() && pending.len() < DOWNLOAD_CONCURRENCY {
            wait_for_run(control).await?;
            let (offset, len) = ranges[next];
            next += 1;
            pending.push_back(tokio::spawn(fetch(offset, len)));
        }

        if pending.is_empty() {
            break;
        }

        // 写入阶段：始终取队首（最早派发 = offset 最小），保证按序落盘。
        let handle = pending.pop_front().expect("pending is non-empty");
        let buf = match handle.await {
            Ok(result) => result,
            Err(join_error) => {
                // 在途任务由 `pending` 的 Drop 中止（R3-08）
                return Err(ProtocolError::ProtocolError(format!(
                    "range read task failed during copy: {join_error}"
                )));
            }
        };
        let buf = match buf {
            Ok(buf) => buf,
            Err(err) => return Err(err),
        };

        // 到写点才检查控制信号：暂停时字节停止增长。
        wait_for_run(control).await?;

        writer
            .write_all(&buf)
            .await
            .map_err(|e| ProtocolError::ProtocolError(format!("write failed during copy: {e}")))?;
        done += buf.len() as u64;
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

/// 分块前的控制检查：`Cancel` 立即中止；`Pause` 挂起等待下一信号；
/// 控制通道关闭（发送端已随任务清理）视同取消。
///
/// `borrow_and_update` 返回的 `Ref` 会在 match 结束后析构，因此先 match
/// 归类、再在 Pause 分支外可变借用 `changed()` 等待，避免借用冲突。
async fn wait_for_run(control: &mut watch::Receiver<TransferControl>) -> Result<(), ProtocolError> {
    loop {
        match *control.borrow_and_update() {
            TransferControl::Cancel => return Err(ProtocolError::TransferCancelled),
            TransferControl::Run => return Ok(()),
            TransferControl::Pause => {}
        }
        control
            .changed()
            .await
            .map_err(|_| ProtocolError::TransferCancelled)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::{Arc, Mutex};

    /// 记录进度帧的共享 sink
    type ProgressLog = Arc<Mutex<Vec<(u64, u64)>>>;

    // ── 句柄池 ──
    // 回归点是逐区间 session.open()：1 GiB 文件会变成 16384 次 OPEN +
    // 16384 次 CLOSE。这里锁定「N 个区间只用 N 个句柄」这一性质。
    #[tokio::test]
    async fn handle_pool_serves_many_borrows_from_a_fixed_set() {
        let pool = HandlePool::with_handles((0..DOWNLOAD_CONCURRENCY as u32).collect::<Vec<u32>>());
        assert_eq!(pool.idle_count().await, DOWNLOAD_CONCURRENCY);

        // 模拟 copy_pipelined 的真实节奏：在途数不超过池大小，分波次借还，
        // 区间总数远多于句柄数——这正是「固定句柄数服务大量区间」的场景
        let waves = 8usize;
        for _ in 0..waves {
            let mut in_flight = Vec::new();
            for _ in 0..DOWNLOAD_CONCURRENCY {
                in_flight.push(pool.acquire().await.expect("池应能借出句柄"));
            }
            assert_eq!(pool.idle_count().await, 0, "全部借出后池必须为空");
            for handle in in_flight {
                pool.release(handle).await;
            }
            assert_eq!(
                pool.idle_count().await,
                DOWNLOAD_CONCURRENCY,
                "归还后池必须回到满状态：句柄不得泄漏，也不得凭空多出"
            );
        }
    }

    // 在途数超过池容量时 acquire 必须返回 None，交给调用方兜底新建，
    // 而不是重复交出同一句柄（那会让两个区间读到同一位置）
    #[tokio::test]
    async fn handle_pool_refuses_to_over_lend() {
        let pool = HandlePool::with_handles(vec![1u32]);
        assert_eq!(pool.acquire().await, Some(1u32));
        assert!(pool.acquire().await.is_none());
    }

    #[tokio::test]
    async fn handle_pool_reports_empty_so_the_caller_can_fall_back() {
        let pool = HandlePool::with_handles(vec![7u32]);
        assert!(pool.acquire().await.is_some());
        // 池空时必须返回 None，调用方据此走「现开一个」的兜底，
        // 而不是拿到重复句柄导致两个区间读同一位置
        assert!(pool.acquire().await.is_none());
        pool.release(7u32).await;
        assert_eq!(pool.acquire().await, Some(7u32));
    }

    // 归还多于借出（任务被中止后又走兜底新建）时池不能无限增长
    #[tokio::test]
    async fn handle_pool_never_grows_beyond_capacity() {
        let pool = HandlePool::with_handles(vec![1u32, 2u32]);
        for extra in 100u32..110 {
            pool.release(extra).await;
        }
        assert_eq!(
            pool.idle_count().await,
            2,
            "池容量是上限，多余归还必须被丢弃"
        );
    }

    #[tokio::test]
    async fn handle_pool_hands_out_distinct_handles_under_concurrency() {
        let pool = Arc::new(HandlePool::with_handles(
            (0..DOWNLOAD_CONCURRENCY as u32).collect::<Vec<u32>>(),
        ));
        let mut tasks = Vec::new();
        for _ in 0..DOWNLOAD_CONCURRENCY {
            let pool = pool.clone();
            tasks.push(tokio::spawn(async move {
                let mut seen = Vec::new();
                for _ in 0..8 {
                    if let Some(h) = pool.acquire().await {
                        seen.push(h);
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                        pool.release(h).await;
                    }
                }
                seen
            }));
        }
        let mut all = Vec::new();
        for t in tasks {
            all.extend(t.await.unwrap());
        }
        let unique: std::collections::HashSet<u32> = all.iter().copied().collect();
        assert_eq!(
            unique.len(),
            DOWNLOAD_CONCURRENCY,
            "同时在途的句柄必须互不相同"
        );
    }

    /// 区间划分必须是连续、不重叠、完整覆盖 [0, total) 的升序切片，
    /// 否则并发下载会把内容写错位置——这是整个并发化里唯一会损坏文件的点。
    #[test]
    fn plan_ranges_covers_total_without_gaps_or_overlap() {
        for &(total, chunk) in &[
            (0u64, 64usize),
            (1, 64),
            (63, 64),
            (64, 64),
            (65, 64),
            (300, 64),
            (CHUNK_SIZE as u64, CHUNK_SIZE),
            (CHUNK_SIZE as u64 * 3 + 17, CHUNK_SIZE),
        ] {
            let ranges = plan_ranges(total, chunk);
            let sum: u64 = ranges.iter().map(|(_, len)| *len as u64).sum();
            assert_eq!(
                sum, total,
                "total={total} chunk={chunk} 区间长度之和必须等于总量"
            );

            let mut cursor = 0u64;
            for (offset, len) in &ranges {
                assert_eq!(*offset, cursor, "区间必须首尾相接，total={total}");
                assert!(*len > 0, "区间长度必须为正，total={total}");
                assert!(*len <= chunk.max(1), "区间长度不得超过分块大小");
                cursor += *len as u64;
            }
        }
    }

    #[test]
    fn plan_ranges_uses_full_chunks_except_the_last() {
        let chunk = 64usize;
        let ranges = plan_ranges(300, chunk);
        assert_eq!(ranges.len(), 5);
        for (offset, len) in &ranges[..4] {
            assert_eq!(*len, chunk, "非末区间必须是满块");
            assert_eq!(
                *offset,
                ranges.iter().position(|r| r == &(*offset, *len)).unwrap() as u64 * chunk as u64
            );
        }
        assert_eq!(ranges[4], (256, 44), "末区间承载余数");
    }

    /// 并发下载：完成顺序打乱也不能影响落盘内容——写入必须按 offset 升序。
    #[tokio::test]
    async fn pipelined_copy_writes_ranges_in_offset_order() {
        let total = 5 * CHUNK_SIZE as u64 + 11;
        // 源数据：每个字节等于其 offset，便于校验错位
        let source: Arc<Vec<u8>> =
            Arc::new((0..total).map(|i| (i % 251) as u8).collect::<Vec<u8>>());
        let expected = source.as_slice().to_vec();

        // 故意让后面的区间先返回，验证写入顺序不依赖完成顺序
        let fetch = move |offset: u64, len: usize| {
            let source = source.clone();
            async move {
                // 反向延迟：offset 越大越先完成
                let rank = (total - offset) / CHUNK_SIZE as u64;
                tokio::time::sleep(std::time::Duration::from_millis(rank * 3)).await;
                Ok(source[offset as usize..offset as usize + len].to_vec())
            }
        };

        let mut sink: Vec<u8> = Vec::new();
        let (_, mut control) = watch::channel(TransferControl::Run);
        let copied = copy_pipelined(total, fetch, &mut sink, &mut control, &mut |_, _| {})
            .await
            .unwrap();

        assert_eq!(copied, total);
        assert_eq!(sink.len() as u64, total, "落盘长度必须等于总量");
        assert_eq!(
            sink,
            expected.as_slice(),
            "并发完成顺序被打乱时，落盘内容仍须与源逐字节一致"
        );
    }

    /// 进度必须单调递增：并发读取下 done 只能按写入顺序累加。
    #[tokio::test]
    async fn pipelined_copy_reports_monotonic_progress() {
        let total = 4 * CHUNK_SIZE as u64;
        let source: Arc<Vec<u8>> = Arc::new(vec![3u8; total as usize]);
        let fetch = move |offset: u64, len: usize| {
            let source = source.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_millis((total - offset) % 7)).await;
                Ok(source[offset as usize..offset as usize + len].to_vec())
            }
        };

        let log: ProgressLog = Arc::new(Mutex::new(Vec::new()));
        let progress_log = log.clone();
        let mut sink: Vec<u8> = Vec::new();
        let (_, mut control) = watch::channel(TransferControl::Run);
        copy_pipelined(total, fetch, &mut sink, &mut control, &mut move |d, t| {
            progress_log.lock().unwrap().push((d, t))
        })
        .await
        .unwrap();

        let frames = log.lock().unwrap().clone();
        assert_eq!(frames.first().unwrap(), &(0, total), "首帧必须是 0/total");
        assert_eq!(frames.last().unwrap().0, total, "末帧必须达到 total");
        assert!(
            frames.windows(2).all(|w| w[0].0 < w[1].0),
            "进度必须严格递增: {frames:?}"
        );
        assert!(
            frames.windows(2).all(|w| w[0].1 == w[1].1),
            "total 在各帧间不得变化: {frames:?}"
        );
    }

    /// 达到并发上限才派发更多区间；观测到的最大在途数应等于并发度。
    #[tokio::test]
    async fn pipelined_copy_keeps_multiple_ranges_in_flight() {
        let ranges_total = 64usize;
        let total = (ranges_total * CHUNK_SIZE) as u64;
        let in_flight = Arc::new(Mutex::new(0usize));
        let peak = Arc::new(Mutex::new(0usize));
        let source: Arc<Vec<u8>> = Arc::new(vec![1u8; total as usize]);

        let fetch = {
            let in_flight = in_flight.clone();
            let peak = peak.clone();
            let source = source.clone();
            move |offset: u64, len: usize| {
                let in_flight = in_flight.clone();
                let peak = peak.clone();
                let source = source.clone();
                async move {
                    let now = {
                        let mut g = in_flight.lock().unwrap();
                        *g += 1;
                        *g
                    };
                    {
                        let mut observed = peak.lock().unwrap();
                        if now > *observed {
                            *observed = now;
                        }
                    }

                    // 每个区间都挂起一段时间：只有真正并发才会堆到上限
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    *in_flight.lock().unwrap() -= 1;
                    Ok(source[offset as usize..offset as usize + len].to_vec())
                }
            }
        };

        let mut sink: Vec<u8> = Vec::new();
        let (_, mut control) = watch::channel(TransferControl::Run);
        let copied = copy_pipelined(total, fetch, &mut sink, &mut control, &mut |_, _| {})
            .await
            .unwrap();

        assert_eq!(copied, total);
        assert_eq!(sink.len(), total as usize);
        let observed_peak = *peak.lock().unwrap();
        assert!(
            observed_peak > 1,
            "串行读取不会提升吞吐；实际峰值在途数 = {observed_peak}，说明仍是串行"
        );
        assert!(
            observed_peak <= DOWNLOAD_CONCURRENCY,
            "在途区间数不得超过并发上限，实际 {observed_peak}"
        );
    }

    #[tokio::test]
    async fn pipelined_copy_propagates_fetch_error_and_aborts_inflight() {
        let total = 16 * CHUNK_SIZE as u64;
        let fetch = |offset: u64, _len: usize| async move {
            if offset >= 4 * CHUNK_SIZE as u64 {
                Err(ProtocolError::ProtocolError(format!("boom at {offset}")))
            } else {
                Ok(vec![0u8; CHUNK_SIZE])
            }
        };

        let mut sink: Vec<u8> = Vec::new();
        let (_, mut control) = watch::channel(TransferControl::Run);
        let err = copy_pipelined(total, fetch, &mut sink, &mut control, &mut |_, _| {})
            .await
            .unwrap_err();

        assert!(
            format!("{err:?}").contains("boom at"),
            "应透传区间读取错误，实际: {err:?}"
        );
    }

    /// 取消必须立刻中止并保持 TransferCancelled 终态语义。
    #[tokio::test]
    async fn pipelined_copy_cancels_without_completing() {
        let total = 64 * CHUNK_SIZE as u64;
        let source: Arc<Vec<u8>> = Arc::new(vec![5u8; total as usize]);
        let (control_tx, mut control) = watch::channel(TransferControl::Run);

        let fetch = move |offset: u64, len: usize| {
            let source = source.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                Ok(source[offset as usize..offset as usize + len].to_vec())
            }
        };

        let mut sink: Vec<u8> = Vec::new();
        let copy = tokio::spawn(async move {
            copy_pipelined(total, fetch, &mut sink, &mut control, &mut |_, _| {}).await
        });

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        control_tx.send(TransferControl::Cancel).unwrap();

        let result = copy.await.unwrap();
        assert!(
            matches!(result, Err(ProtocolError::TransferCancelled)),
            "取消必须返回 TransferCancelled，实际: {result:?}"
        );
    }

    // ── R3-08：在途区间任务必须被中止 ──
    // 泄漏的两条路径都不经过 `handle.await` 的错误分支：`wait_for_run` 的
    // `?`（取消）和本地 `write_all` 的 `?`。它们过去只 drop 掉
    // `VecDeque<JoinHandle>`，任务转为游离态继续发 READ、继续占句柄。
    //
    // 观测方式：fetch future 持有一个 Drop 计数器，任务被 abort 时随 future
    // 一起析构 → 计数增长。据此断言「已派发数 == 被中止数」，不依赖时序。
    struct DropCount(Arc<Mutex<usize>>);

    impl Drop for DropCount {
        fn drop(&mut self) {
            *self.0.lock().unwrap() += 1;
        }
    }

    /// abort 只置取消标志，future 的析构发生在下一次调度。这里有界地让出执行权
    /// 直到计数收敛，不依赖任何固定 sleep 时长；无中止时它会跑满循环后返回现值。
    async fn wait_for_drops(dropped: &Arc<Mutex<usize>>, expected: usize) -> usize {
        for _ in 0..10_000 {
            let now = *dropped.lock().unwrap();
            if now >= expected {
                return now;
            }
            tokio::task::yield_now().await;
        }
        *dropped.lock().unwrap()
    }

    /// 第一次 `poll_write` 就失败的本地 writer（磁盘满 / 目录被删）。
    struct FailingWriter;

    impl tokio::io::AsyncWrite for FailingWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Err(std::io::Error::other("no space left on device")))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    /// fetch future 的类型：按 offset 分支需要装箱
    type RangeFetch =
        std::pin::Pin<Box<dyn Future<Output = Result<Vec<u8>, ProtocolError>> + Send>>;

    /// 队首区间立刻返回，其余区间永不自行完成（只有 abort 能结束它们）。
    fn head_only_fetch(
        started: Arc<Mutex<usize>>,
        dropped: Arc<Mutex<usize>>,
    ) -> impl Fn(u64, usize) -> RangeFetch {
        move |offset: u64, len: usize| {
            let started = started.clone();
            let dropped = dropped.clone();
            Box::pin(async move {
                *started.lock().unwrap() += 1;
                let _live = DropCount(dropped);
                if offset == 0 {
                    Ok(vec![0u8; len])
                } else {
                    std::future::pending::<()>().await;
                    unreachable!("只有 abort 能让挂起的区间任务结束")
                }
            })
        }
    }

    /// R3-08 回归：取消发生在 `pending` 非空时（派发阶段的 `wait_for_run`），
    /// 在途区间任务必须全部被 abort。
    #[tokio::test]
    async fn pipelined_copy_aborts_inflight_ranges_on_cancel() {
        let total = 16 * CHUNK_SIZE as u64;
        let started: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let dropped: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let head_ready = Arc::new(tokio::sync::Notify::new());
        let (control_tx, mut control) = watch::channel(TransferControl::Run);

        let fetch = {
            let started = started.clone();
            let dropped = dropped.clone();
            let head_ready = head_ready.clone();
            move |offset: u64, len: usize| {
                let started = started.clone();
                let dropped = dropped.clone();
                let head_ready = head_ready.clone();
                Box::pin(async move {
                    *started.lock().unwrap() += 1;
                    let _live = DropCount(dropped);
                    if offset == 0 {
                        // 队首区间等测试放行，好让拷贝循环走到派发阶段的取消检查
                        head_ready.notified().await;
                        Ok(vec![0u8; len])
                    } else {
                        std::future::pending::<()>().await;
                        unreachable!("只有 abort 能让挂起的区间任务结束")
                    }
                }) as std::pin::Pin<Box<dyn Future<Output = _> + Send>>
            }
        };

        let mut sink: Vec<u8> = Vec::new();
        let copy = tokio::spawn(async move {
            copy_pipelined(total, fetch, &mut sink, &mut control, &mut |_, _| {}).await
        });

        // 等到在途数打满：此刻 pending 非空，取消只能从派发阶段的 `?` 退出
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while *started.lock().unwrap() < DOWNLOAD_CONCURRENCY {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("在途区间数迟迟未打满");
        let started_count = *started.lock().unwrap();
        assert_eq!(started_count, DOWNLOAD_CONCURRENCY);

        control_tx.send(TransferControl::Cancel).unwrap();
        // 放行队首区间，让循环推进到下一轮的取消检查
        head_ready.notify_one();

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), copy)
            .await
            .expect("取消后拷贝必须收尾")
            .unwrap();
        assert!(
            matches!(result, Err(ProtocolError::TransferCancelled)),
            "取消必须返回 TransferCancelled，实际: {result:?}"
        );

        let dropped_count = wait_for_drops(&dropped, started_count).await;
        assert_eq!(
            dropped_count, started_count,
            "取消后不得有在途区间任务存活：已派发 {started_count} 个，仅 {dropped_count} 个被中止"
        );
    }

    /// R3-08 回归：本地写失败的 `?` 提前返回时，在途区间任务必须全部被 abort。
    #[tokio::test]
    async fn pipelined_copy_aborts_inflight_ranges_on_write_error() {
        let total = 16 * CHUNK_SIZE as u64;
        let started: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let dropped: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let fetch = head_only_fetch(started.clone(), dropped.clone());

        let (_, mut control) = watch::channel(TransferControl::Run);
        let mut writer = FailingWriter;
        let err = copy_pipelined(total, fetch, &mut writer, &mut control, &mut |_, _| {})
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("write failed during copy"),
            "应透传本地写错误，实际: {err:?}"
        );

        let started_count = *started.lock().unwrap();
        assert_eq!(
            started_count, DOWNLOAD_CONCURRENCY,
            "写失败前应已派发满并发度的区间"
        );
        let dropped_count = wait_for_drops(&dropped, started_count).await;
        assert_eq!(
            dropped_count, started_count,
            "写失败后不得有在途区间任务存活：已派发 {started_count} 个，仅 {dropped_count} 个被中止"
        );
    }

    /// 源在传输期间变短时报错，不留下被截断却当成功的文件。
    #[tokio::test]
    async fn pipelined_copy_rejects_short_source() {
        let total = 8 * CHUNK_SIZE as u64;
        let fetch = |offset: u64, len: usize| async move {
            if offset == 0 {
                Ok(vec![0u8; len])
            } else {
                // 假装远端只剩一半
                Err(ProtocolError::ProtocolError(format!(
                    "range read failed at {offset} (+{len})"
                )))
            }
        };

        let mut sink: Vec<u8> = Vec::new();
        let (_, mut control) = watch::channel(TransferControl::Run);
        assert!(
            copy_pipelined(total, fetch, &mut sink, &mut control, &mut |_, _| {})
                .await
                .is_err()
        );
    }

    /// 等待拷贝循环推进到至少 `min_done` 字节
    async fn wait_for_progress(log: &ProgressLog, min_done: u64) {
        for _ in 0..1000 {
            let done = log.lock().unwrap().last().map(|frame| frame.0).unwrap_or(0);
            if done >= min_done {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("拷贝循环迟迟未产生进度");
    }

    fn last_done(log: &ProgressLog) -> u64 {
        log.lock().unwrap().last().map(|frame| frame.0).unwrap_or(0)
    }

    #[tokio::test]
    async fn copy_reports_monotonic_progress_and_exact_final_total() {
        let payload = vec![7u8; 1000];
        let mut reader: &[u8] = payload.as_slice();
        let mut writer: Vec<u8> = Vec::new();
        let mut calls: Vec<(u64, u64)> = Vec::new();
        let (_, mut control) = watch::channel(TransferControl::Run);

        let copied = copy_with_progress(
            &mut reader,
            &mut writer,
            1000,
            128,
            &mut control,
            &mut |done, total| calls.push((done, total)),
        )
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
        let (_, mut control) = watch::channel(TransferControl::Run);

        let copied = copy_with_progress(
            &mut reader,
            &mut writer,
            0,
            64,
            &mut control,
            &mut |d, t| last = (d, t),
        )
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
        let (_, mut control) = watch::channel(TransferControl::Run);

        let err = copy_with_progress(
            &mut reader,
            &mut writer,
            1000,
            64,
            &mut control,
            &mut |_, _| {},
        )
        .await
        .unwrap_err();

        assert!(
            format!("{err:?}").contains("source changed"),
            "实际错误: {err:?}"
        );
    }

    /// 大文件拷贝中途暂停：字节停止增长、拷贝不结束；恢复后继续直到完成。
    #[tokio::test]
    async fn copy_pauses_between_chunks_and_resumes() {
        let (mut producer, mut consumer) = tokio::io::duplex(64);
        let (control_tx, mut control) = watch::channel(TransferControl::Run);
        let payload = vec![7u8; 4096];
        let total = payload.len() as u64;

        // 生产端按 20ms/块节拍写入：内存 duplex 上的拷贝会在 wait_for_progress
        // 的一个采样窗口内跑完全程，暂停/取消必须在拷贝中途送达才有意义。
        // 暂停后拷贝循环不再读取，64 字节缓冲写满后 write_all 挂起。
        let producer_task = tokio::spawn(async move {
            for chunk in payload.chunks(64) {
                if producer.write_all(chunk).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        });

        let log: ProgressLog = Arc::new(Mutex::new(Vec::new()));
        let progress_log = log.clone();
        let copy_task = tokio::spawn(async move {
            let mut sink: Vec<u8> = Vec::new();
            let mut progress = move |done: u64, total: u64| {
                progress_log.lock().unwrap().push((done, total));
            };
            let result = copy_with_progress(
                &mut consumer,
                &mut sink,
                total,
                64,
                &mut control,
                &mut progress,
            )
            .await;
            (result, sink)
        });

        wait_for_progress(&log, 256).await;
        control_tx.send(TransferControl::Pause).unwrap();

        // 让在途分块落定后两次采样：暂停期间进度必须完全停止
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let paused_a = last_done(&log);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let paused_b = last_done(&log);

        assert!(paused_a >= 256, "暂停前应已拷贝若干分块");
        assert_eq!(paused_a, paused_b, "暂停期间字节不得继续增长");
        assert!(!copy_task.is_finished(), "暂停中的拷贝不得提前结束");
        assert!(!producer_task.is_finished(), "暂停期间生产端不得写完");

        control_tx.send(TransferControl::Run).unwrap();
        let (result, sink) = tokio::time::timeout(std::time::Duration::from_secs(5), copy_task)
            .await
            .unwrap()
            .unwrap();
        let copied = result.unwrap();

        assert_eq!(copied, total, "恢复后应从已传字节继续到完成");
        assert_eq!(sink.len() as u64, total);
        producer_task.await.unwrap();
    }

    /// 大文件拷贝中途取消：返回 TransferCancelled、不产生完成结果。
    #[tokio::test]
    async fn copy_cancels_between_chunks_without_completing() {
        let (mut producer, mut consumer) = tokio::io::duplex(64);
        let (control_tx, mut control) = watch::channel(TransferControl::Run);
        let payload = vec![3u8; 4096];
        let total = payload.len() as u64;

        // 与暂停测试同款节拍：保证取消信号在拷贝中途（而非结束后）送达
        let producer_task = tokio::spawn(async move {
            for chunk in payload.chunks(64) {
                if producer.write_all(chunk).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        });

        let log: ProgressLog = Arc::new(Mutex::new(Vec::new()));
        let progress_log = log.clone();
        let copy_task = tokio::spawn(async move {
            let mut sink: Vec<u8> = Vec::new();
            let mut progress = move |done: u64, total: u64| {
                progress_log.lock().unwrap().push((done, total));
            };
            let result = copy_with_progress(
                &mut consumer,
                &mut sink,
                total,
                64,
                &mut control,
                &mut progress,
            )
            .await;
            (result, sink)
        });

        wait_for_progress(&log, 256).await;
        control_tx.send(TransferControl::Cancel).unwrap();

        let (result, sink) = tokio::time::timeout(std::time::Duration::from_secs(5), copy_task)
            .await
            .unwrap()
            .unwrap();

        let err = result.unwrap_err();
        assert!(
            matches!(err, ProtocolError::TransferCancelled),
            "实际错误: {err:?}"
        );
        assert!(sink.len() < total as usize, "取消时不得写入完整文件");
        producer_task.await.ok();
    }

    /// 控制通道在暂停等待期间关闭（发送端已清理）时视同取消，拷贝不得永久挂起。
    #[tokio::test]
    async fn copy_aborts_when_control_channel_closes_while_paused() {
        let (control_tx, mut control) = watch::channel(TransferControl::Pause);

        let copy_task = tokio::spawn(async move {
            let payload = vec![5u8; 1000];
            let mut reader: &[u8] = payload.as_slice();
            let mut writer: Vec<u8> = Vec::new();
            let result = copy_with_progress(
                &mut reader,
                &mut writer,
                1000,
                64,
                &mut control,
                &mut |_, _| {},
            )
            .await;
            (result, writer)
        });

        // 拷贝任务在首个分块前挂起；随后清理发送端，必须以取消收尾而非永久挂起
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(control_tx);

        let (result, writer) = tokio::time::timeout(std::time::Duration::from_secs(5), copy_task)
            .await
            .unwrap()
            .unwrap();

        let err = result.unwrap_err();
        assert!(matches!(err, ProtocolError::TransferCancelled));
        assert!(writer.is_empty(), "未恢复前不得写入任何字节");
    }
}
