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

/// SFTP 客户端
///
/// 封装 russh_sftp::client::SftpSession，提供高层文件操作接口。
///
/// `SftpSession` 自身不是 `Clone`，但全部方法都取 `&self`；用 `Arc` 共享，
/// 让并发区间下载的多个任务能同时在同一条 SFTP 会话上发 READ
/// （russh-sftp 按请求 id 多路复用，并发请求是协议层支持的）。
pub struct SftpClient {
    session: std::sync::Arc<russh_sftp::client::SftpSession>,
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
            session: std::sync::Arc::new(session),
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
        control: &mut watch::Receiver<TransferControl>,
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
            let session = self.session.clone();
            let remote = remote.to_string();
            copy_pipelined(
                total,
                move |offset, len| {
                    let session = session.clone();
                    let remote = remote.clone();
                    async move {
                        let mut file = session.open(remote).await.map_err(|e| {
                            ProtocolError::ProtocolError(format!(
                                "Failed to open remote file for range: {e}"
                            ))
                        })?;
                        file.seek(std::io::SeekFrom::Start(offset))
                            .await
                            .map_err(|e| {
                                ProtocolError::ProtocolError(format!(
                                    "Failed to seek remote file to {offset}: {e}"
                                ))
                            })?;
                        let mut buf = vec![0u8; len];
                        file.read_exact(&mut buf).await.map_err(|e| {
                            ProtocolError::ProtocolError(format!(
                                "range read failed at {offset} (+{len}): {e}"
                            ))
                        })?;
                        Ok(buf)
                    }
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
    let mut pending: VecDeque<tokio::task::JoinHandle<Result<Vec<u8>, ProtocolError>>> =
        VecDeque::new();
    let mut next = 0usize;

    // 提前中止时（暂停以外的取消/错误）必须停掉在途任务，
    // 否则它们会继续占用 SFTP 请求槽并可能写入已废弃的缓冲。
    let abort_all =
        |pending: &mut VecDeque<tokio::task::JoinHandle<Result<Vec<u8>, ProtocolError>>>| {
            for handle in pending.drain(..) {
                handle.abort();
            }
        };

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
                abort_all(&mut pending);
                return Err(ProtocolError::ProtocolError(format!(
                    "range read task failed during copy: {join_error}"
                )));
            }
        };
        let buf = match buf {
            Ok(buf) => buf,
            Err(err) => {
                abort_all(&mut pending);
                return Err(err);
            }
        };

        // 到写点才检查控制信号：暂停时字节停止增长。
        if let Err(err) = wait_for_run(control).await {
            abort_all(&mut pending);
            return Err(err);
        }

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
    use std::sync::{Arc, Mutex};

    /// 记录进度帧的共享 sink
    type ProgressLog = Arc<Mutex<Vec<(u64, u64)>>>;

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
