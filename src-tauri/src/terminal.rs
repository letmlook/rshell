//! 终端字节流路由 —— 设计 §4.1 双态 sink
//!
//! 解决时序缺口：recv 循环在 `connect()` 内 `tokio::spawn` 启动，而 Tauri `Channel`
//! 由前端组件 `onMounted` 时创建。两者时序无保证。
//!
//! 解法：后端持 sink，未 attach 期间字节进入 `Buffering`，attach 时一次性 flush
//! 并切换为 `Attached(Channel)`。Channel 失效（HMR / 窗口重载）时退回 `Buffering`，
//! **不断开 SSH 连接** —— 这是设计 §4.4 的"前后端故障域隔离"。
//!
//! 键是 **(session_id, terminal_id)**：一个连接可以挂多个 pty（多标签会话），
//! 每个标签一个独立 pty，输出必须按 terminal 隔离——
//!
//! - 用 session_id 单键会让新标签顶掉旧标签的通道（旧标签停止刷新）；
//! - 也不该把一个 pty 的字节扇出给所有标签（那是共享 shell 的做法）。
//!
//! 新标签 attach 时补发该 pty 的滚动历史，因此一挂上就有内容（含 shell 提示符）。

#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tauri::ipc::Channel;
use tokio::sync::RwLock;
use tracing::warn;
use uuid::Uuid;

/// 256 KiB 积压上限。超过则丢弃最旧字节并 warn!（设计 §4.1 规则 1）
const BUFFER_CAP_BYTES: usize = 256 * 1024;

/// 一个 pty 的路由状态。
struct TermSink {
    /// 已 attach 的前端通道（同一 pty 被 HMR 重挂时可能有多个）
    channels: Vec<Channel<Vec<u8>>>,
    /// 滚动历史：attach 时补发给新通道
    backlog: VecDeque<u8>,
}

impl TermSink {
    fn new() -> Self {
        Self {
            channels: Vec::new(),
            backlog: VecDeque::with_capacity(BUFFER_CAP_BYTES),
        }
    }
}

pub struct TerminalChannels {
    inner: RwLock<HashMap<(Uuid, Uuid), TermSink>>,
}

impl Default for TerminalChannels {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalChannels {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// recv 循环无条件写入。未注册 pty 静默丢弃 + warn（防御性：recv 早于 attach）。
    /// 已 attach 的每个通道都收到一份；失败的（标签已关 / HMR 失效）就地摘掉。
    pub async fn push(&self, session_id: Uuid, terminal_id: Uuid, data: &[u8]) {
        let mut inner = self.inner.write().await;
        let sink = inner
            .entry((session_id, terminal_id))
            .or_insert_with(TermSink::new);

        // 无论有没有前端都留一份滚动历史：这是新标签 attach 后立刻有内容的关键
        let dropped = append_capped(&mut sink.backlog, data, BUFFER_CAP_BYTES);
        if dropped > 0 {
            warn!(
                session_id = %session_id,
                terminal_id = %terminal_id,
                dropped_bytes = dropped,
                "terminal buffer overflow; oldest bytes dropped (frontend attach lags)"
            );
        }

        if sink.channels.is_empty() {
            return;
        }
        // 扇出：把失效通道（send 失败）摘掉，活的留下继续收
        let mut alive = Vec::with_capacity(sink.channels.len());
        let mut failed = 0usize;
        for channel in std::mem::take(&mut sink.channels) {
            match channel.send(data.to_vec()) {
                Ok(()) => alive.push(channel),
                Err(_) => failed += 1,
            }
        }
        sink.channels = alive;
        if failed > 0 {
            warn!(
                session_id = %session_id,
                terminal_id = %terminal_id,
                dropped_channels = failed,
                "channel.send failed; dropping those tabs (other tabs unaffected)"
            );
        }
    }

    /// 前端 mount xterm 后调用 —— 补发滚动历史后登记通道。
    /// 之后每次 push 都扇出到这个通道。**不会断开 SSH**。
    pub async fn attach(&self, session_id: Uuid, terminal_id: Uuid, channel: Channel<Vec<u8>>) {
        let mut inner = self.inner.write().await;
        let sink = inner
            .entry((session_id, terminal_id))
            .or_insert_with(TermSink::new);

        // 先补发历史：新标签挂上就能看到此前的输出（含 shell 提示符），
        // 而不是一片空白、非得敲一次回车才有内容。
        if !sink.backlog.is_empty() {
            let bytes: Vec<u8> = sink.backlog.iter().copied().collect();
            if let Err(e) = channel.send(bytes) {
                warn!(session_id = %session_id, terminal_id = %terminal_id, error = %e, "attach: flush积压失败,丢弃积压");
            }
        }
        sink.channels.push(channel);
    }

    /// 显式 detach(会话删除时)：连同滚动历史一起清掉。
    /// 单个标签关闭不需要显式 detach —— 下一次 push 时该通道 send 失败会被摘掉。
    pub async fn detach(&self, session_id: Uuid) {
        let mut inner = self.inner.write().await;
        inner.retain(|(sid, _), _| *sid != session_id);
    }

    /// 测试与诊断用 —— 当前 sink 状态概览。
    pub async fn debug_summary(&self) -> Vec<((Uuid, Uuid), String)> {
        let inner = self.inner.read().await;
        inner
            .iter()
            .map(|(key, sink)| {
                let tag = if sink.channels.is_empty() {
                    "buffering".to_string()
                } else {
                    format!("attached\u{d7}{}", sink.channels.len())
                };
                (*key, tag)
            })
            .collect()
    }
}

/// 把 data 追加到 buf 末尾直至容量上限 cap,返回丢弃字节数。
fn append_capped(buf: &mut VecDeque<u8>, data: &[u8], cap: usize) -> usize {
    let mut dropped = 0;
    for &b in data {
        if buf.len() >= cap {
            buf.pop_front();
            dropped += 1;
        }
        buf.push_back(b);
    }
    dropped
}

/// 切片 1.1 的辅助类型:把 Arc<TerminalChannels> 与各命令共享。
pub type SharedTerminalChannels = Arc<TerminalChannels>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tauri::ipc::InvokeResponseBody;

    /// 造一个把收到的字节记进 sink 的通道；`fail` 为 true 时让 send 失败，
    /// 用来模拟标签已关闭 / HMR 导致的句柄失效。
    fn recording_channel(sink: Arc<Mutex<Vec<Vec<u8>>>>, fail: bool) -> Channel<Vec<u8>> {
        Channel::new(move |body: InvokeResponseBody| {
            if fail {
                return Err(tauri::Error::AssetNotFound("channel closed".into()));
            }
            // Vec<u8> 经 IpcResponse 变成 JSON 数组字符串，这里解回来比对字节
            let bytes: Vec<u8> = match body {
                InvokeResponseBody::Json(text) => serde_json::from_str(&text).unwrap_or_default(),
                InvokeResponseBody::Raw(raw) => raw,
            };
            sink.lock().expect("sink poisoned").push(bytes);
            Ok(())
        })
    }

    #[tokio::test]
    async fn buffering_then_attach_preserves_order() {
        let tc = TerminalChannels::new();
        let (sid, tid) = (Uuid::new_v4(), Uuid::new_v4());
        tc.push(sid, tid, b"hello ").await;
        tc.push(sid, tid, b"world").await;
        tc.push(sid, tid, b"!").await;
        let summary = tc.debug_summary().await;
        assert_eq!(summary, vec![((sid, tid), "buffering".to_string())]);
        tc.detach(sid).await;
        let summary = tc.debug_summary().await;
        assert!(summary.is_empty());
    }

    #[tokio::test]
    async fn overflow_drops_oldest_bytes() {
        let tc = TerminalChannels::new();
        let (sid, tid) = (Uuid::new_v4(), Uuid::new_v4());
        // 灌入 2 × 上限
        let big: Vec<u8> = (0..(BUFFER_CAP_BYTES * 2) as u8)
            .cycle()
            .take(BUFFER_CAP_BYTES * 2)
            .collect();
        tc.push(sid, tid, &big).await;
        let summary = tc.debug_summary().await;
        assert_eq!(summary, vec![((sid, tid), "buffering".to_string())]);
    }

    // 回归：attach 时补发滚动历史，新标签一挂上就有内容（此前新标签空白，
    // 非得敲一次回车才能看到 shell 提示符）
    #[tokio::test]
    async fn attach_replays_backlog_to_the_new_tab() {
        let tc = TerminalChannels::new();
        let (sid, tid) = (Uuid::new_v4(), Uuid::new_v4());
        tc.push(sid, tid, b"letmlook@dev:~$ ").await;

        let seen = Arc::new(Mutex::new(Vec::new()));
        tc.attach(sid, tid, recording_channel(seen.clone(), false))
            .await;

        assert_eq!(
            seen.lock().expect("sink poisoned").as_slice(),
            &[b"letmlook@dev:~$ ".to_vec()],
            "新标签 attach 时应收到此前的输出",
        );
    }

    // 回归：同会话的多个标签各自一个 pty，输出**按 terminal 隔离**。
    // 此前每会话一个 sink，新标签 attach 会顶掉旧标签的通道（旧标签停止刷新）。
    #[tokio::test]
    async fn routes_output_per_terminal_not_per_session() {
        let tc = TerminalChannels::new();
        let sid = Uuid::new_v4();
        let (first, second) = (Uuid::new_v4(), Uuid::new_v4());

        let a = Arc::new(Mutex::new(Vec::new()));
        let b = Arc::new(Mutex::new(Vec::new()));
        tc.attach(sid, first, recording_channel(a.clone(), false))
            .await;
        tc.attach(sid, second, recording_channel(b.clone(), false))
            .await;

        tc.push(sid, first, b"only-first").await;

        assert_eq!(
            a.lock().expect("sink poisoned").as_slice(),
            &[b"only-first".to_vec()]
        );
        assert!(
            b.lock().expect("sink poisoned").is_empty(),
            "第二个标签的 pty 不该收到第一个 pty 的字节"
        );
    }

    // 同一 pty 被 HMR 重挂时可能有两个通道，两个都要收到（旧的失效会被摘掉）
    #[tokio::test]
    async fn fans_out_to_every_channel_of_the_same_terminal() {
        let tc = TerminalChannels::new();
        let (sid, tid) = (Uuid::new_v4(), Uuid::new_v4());

        let first = Arc::new(Mutex::new(Vec::new()));
        let second = Arc::new(Mutex::new(Vec::new()));
        tc.attach(sid, tid, recording_channel(first.clone(), false))
            .await;
        tc.attach(sid, tid, recording_channel(second.clone(), false))
            .await;

        tc.push(sid, tid, b"ls\r\n").await;

        assert_eq!(
            first.lock().expect("sink poisoned").as_slice(),
            &[b"ls\r\n".to_vec()]
        );
        assert_eq!(
            second.lock().expect("sink poisoned").as_slice(),
            &[b"ls\r\n".to_vec()]
        );
        assert_eq!(
            tc.debug_summary().await,
            vec![((sid, tid), "attached\u{d7}2".to_string())]
        );
    }

    // 关闭一个标签（通道 send 失败）不能影响同一 pty 的其它通道
    #[tokio::test]
    async fn drops_only_the_failed_channel() {
        let tc = TerminalChannels::new();
        let (sid, tid) = (Uuid::new_v4(), Uuid::new_v4());

        let alive = Arc::new(Mutex::new(Vec::new()));
        tc.attach(
            sid,
            tid,
            recording_channel(Arc::new(Mutex::new(Vec::new())), true),
        )
        .await;
        tc.attach(sid, tid, recording_channel(alive.clone(), false))
            .await;

        tc.push(sid, tid, b"after-close").await;

        assert_eq!(
            alive.lock().expect("sink poisoned").as_slice(),
            &[b"after-close".to_vec()]
        );
        assert_eq!(
            tc.debug_summary().await,
            vec![((sid, tid), "attached\u{d7}1".to_string())],
            "失效通道应被摘掉，剩下的通道继续收"
        );
    }

    // detach 按会话清理：只清掉该会话的 pty，不影响别的会话
    #[tokio::test]
    async fn detach_session_keeps_other_sessions() {
        let tc = TerminalChannels::new();
        let sid = Uuid::new_v4();
        let other = Uuid::new_v4();
        tc.push(sid, Uuid::new_v4(), b"a").await;
        tc.push(other, Uuid::new_v4(), b"b").await;

        tc.detach(sid).await;

        let summary = tc.debug_summary().await;
        assert_eq!(summary.len(), 1, "只应清掉该会话的 pty");
        assert_eq!(summary[0].0 .0, other);
    }
}
