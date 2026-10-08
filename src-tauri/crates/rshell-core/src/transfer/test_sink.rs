//! R2-T2：测试用 fake sink + source。
//!
//! `FakeSink` 把 `TransferSink` 的全部操作映射到内存 `HashMap<path, bytes>`，
//! 加几个可编程的开关：commit 失败、cleanup 失败、晚冲突。
//!
//! `FakeSource` 把 `AsyncRead` 包成一个 `Vec<u8>`，便于喂固定载荷。
//!
//! 两者的设计目标：让 staged lifecycle 的每一条分支都能用一个 fixture
//! 跑出来，不依赖真实 SFTP 服务器。

#![cfg(test)]

use rshell_protocol::ssh::sftp::{CommitOutcome, CommitStrategy, TransferSink};
use rshell_protocol::ProtocolError;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// 内存文件系统 —— path → 已写入字节。
#[derive(Default)]
pub struct FakeFiles {
    pub files: HashMap<String, Vec<u8>>,
}

impl FakeFiles {
    pub fn with_content(entries: &[(&'static str, &'static [u8])]) -> Self {
        let mut files = HashMap::new();
        for (path, content) in entries {
            files.insert((*path).to_string(), content.to_vec());
        }
        Self { files }
    }

    pub fn get(&self, path: &str) -> Option<&Vec<u8>> {
        self.files.get(path)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(path.to_string(), bytes);
    }
}

/// safe_commit 进入后立刻触发的副作用类型 —— 用来模拟「commit 期间取消到达」。
/// 签名：传入 from / to 路径；测试可在此设置 cancel_decision 等。
type CommitHook = Box<dyn Fn(&str, &str) + Send + Sync>;

/// 测试用 sink —— 内存映射 + 可编程失败开关。
#[derive(Clone)]
pub struct FakeSink {
    pub files: Arc<Mutex<FakeFiles>>,
    pub exclusive_open_should_fail: Arc<AtomicBool>,
    pub commit_should_fail: Arc<AtomicBool>,
    pub cleanup_should_fail: Arc<AtomicBool>,
    /// commit 调用次数 —— 用于断言「未发生 commit」
    pub commit_calls: Arc<AtomicUsize>,
    /// cleanup 调用次数
    pub cleanup_calls: Arc<AtomicUsize>,
    /// 「目标存在」开关 —— 控制晚冲突
    pub target_exists_override: Arc<Mutex<Option<bool>>>,
    /// safe_commit 进入后立刻触发的副作用 —— 用来模拟「commit 期间取消到达」。
    /// `None` 表示不触发。
    pub on_commit: Arc<Mutex<Option<CommitHook>>>,
}

impl Default for FakeSink {
    fn default() -> Self {
        Self {
            files: Arc::new(Mutex::new(FakeFiles::default())),
            exclusive_open_should_fail: Arc::new(AtomicBool::new(false)),
            commit_should_fail: Arc::new(AtomicBool::new(false)),
            cleanup_should_fail: Arc::new(AtomicBool::new(false)),
            commit_calls: Arc::new(AtomicUsize::new(0)),
            cleanup_calls: Arc::new(AtomicUsize::new(0)),
            target_exists_override: Arc::new(Mutex::new(None)),
            on_commit: Arc::new(Mutex::new(None)),
        }
    }
}

impl FakeSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_target_existing(self, exists: bool) -> Self {
        *self.target_exists_override.lock().unwrap() = Some(exists);
        self
    }

    pub fn with_commit_failing(self) -> Self {
        self.commit_should_fail.store(true, AtomicOrdering::SeqCst);
        self
    }

    pub fn with_cleanup_failing(self) -> Self {
        self.cleanup_should_fail.store(true, AtomicOrdering::SeqCst);
        self
    }

    /// 安装一个「safe_commit 期间触发」的回调 —— 由 staged lifecycle
    /// 在 commit CAS 时看见，用于模拟 commit 期间取消到达。
    pub fn with_on_commit<F>(self, f: F) -> Self
    where
        F: Fn(&str, &str) + Send + Sync + 'static,
    {
        *self.on_commit.lock().unwrap() = Some(Box::new(f));
        self
    }

    pub fn with_exclusive_open_failing(self) -> Self {
        self.exclusive_open_should_fail
            .store(true, AtomicOrdering::SeqCst);
        self
    }

    pub fn snapshot(&self, path: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(path).cloned()
    }
}

#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
impl TransferSink for FakeSink {
    async fn exists(&self, path: &str) -> bool {
        if let Some(forced) = *self.target_exists_override.lock().unwrap() {
            return forced;
        }
        self.files.lock().unwrap().contains(path)
    }

    async fn exclusive_open_write(
        &self,
        path: &str,
    ) -> Result<Box<dyn AsyncWrite + Unpin + Send>, ProtocolError> {
        if self.exclusive_open_should_fail.load(AtomicOrdering::SeqCst) {
            return Err(ProtocolError::ProtocolError(format!(
                "fake exclusive open failure for {path}"
            )));
        }
        // 独占：已存在即失败
        {
            let mut files = self.files.lock().unwrap();
            if files.contains(path) {
                return Err(ProtocolError::ProtocolError(format!(
                    "fake exclusive collision for {path}"
                )));
            }
            files.insert(path, Vec::new());
        }
        Ok(Box::new(FakeWrite {
            path: path.to_string(),
            files: self.files.clone(),
        }))
    }

    async fn open_read(
        &self,
        path: &str,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, ProtocolError> {
        let bytes = {
            let files = self.files.lock().unwrap();
            files.get(path).cloned()
        };
        match bytes {
            Some(b) => Ok(Box::new(FakeRead { bytes: b, pos: 0 })),
            None => Err(ProtocolError::ProtocolError(format!(
                "fake open_read: not found {path}"
            ))),
        }
    }

    async fn safe_commit(
        &self,
        from: &str,
        to: &str,
        _caps: rshell_protocol::ssh::sftp::SftpCapabilities,
    ) -> Result<CommitOutcome, ProtocolError> {
        self.commit_calls.fetch_add(1, AtomicOrdering::SeqCst);
        // 在 commit 真正生效之前，触发测试回调 —— 让测试在 commit 期间
        // 设置 cancel_decision 标志，模拟「commit 期间取消到达」。
        if let Some(hook) = self.on_commit.lock().unwrap().as_ref() {
            hook(from, to);
        }
        if self.commit_should_fail.load(AtomicOrdering::SeqCst) {
            return Err(ProtocolError::TransferCommitFailed {
                path: to.to_string(),
                reason: "fake commit failure".into(),
            });
        }
        let mut files = self.files.lock().unwrap();
        let bytes = files.files.remove(from).ok_or_else(|| {
            ProtocolError::ProtocolError(format!("fake commit: temp missing {from}"))
        })?;
        files.insert(to, bytes);
        Ok(CommitOutcome {
            strategy: CommitStrategy::StandardRename,
        })
    }

    async fn try_remove(&self, path: &str) -> Result<(), ProtocolError> {
        self.cleanup_calls.fetch_add(1, AtomicOrdering::SeqCst);
        if self.cleanup_should_fail.load(AtomicOrdering::SeqCst) {
            return Err(ProtocolError::TransferCleanupFailed {
                path: path.to_string(),
                reason: "fake cleanup failure".into(),
            });
        }
        let mut files = self.files.lock().unwrap();
        files.files.remove(path);
        Ok(())
    }

    fn is_remote(&self) -> bool {
        false
    }
}

/// 写入侧：把写入的字节追加到 `files[path]`。
struct FakeWrite {
    path: String,
    files: Arc<Mutex<FakeFiles>>,
}

impl AsyncWrite for FakeWrite {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let me = self.get_mut();
        let mut files = me.files.lock().unwrap();
        let entry = files.files.entry(me.path.clone()).or_default();
        entry.extend_from_slice(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// 读取侧：把 `bytes` 按 offset 读出来。
pub struct FakeRead {
    pub bytes: Vec<u8>,
    pub pos: usize,
}

impl AsyncRead for FakeRead {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let remaining = &self.bytes[self.pos..];
        if remaining.is_empty() {
            return std::task::Poll::Ready(Ok(()));
        }
        let n = remaining.len().min(buf.remaining());
        buf.put_slice(&remaining[..n]);
        self.pos += n;
        std::task::Poll::Ready(Ok(()))
    }
}
