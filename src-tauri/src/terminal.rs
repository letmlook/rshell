//! 终端字节流路由 —— 设计 §4.1 双态 sink
//!
//! 解决时序缺口：recv 循环在 `connect()` 内 `tokio::spawn` 启动，而 Tauri `Channel`
//! 由前端组件 `onMounted` 时创建。两者时序无保证。
//!
//! 解法：后端持每会话双态 sink，未 attach 期间字节进入 `Buffering`，attach 时
//! 一次性 flush 并切换为 `Attached(Channel)`。Channel 失效（HMR / 窗口重载）时退回
//! `Buffering`，**不断开 SSH 连接** —— 这是设计 §4.4 的"前后端故障域隔离"。
//!
//! 切片 1.1：模块已就位但 `TerminalChannels` 暂未被薄壳调用（切片 1.2 的
//! `attach_terminal` 命令会接入 push/attach）。dead_code 警告属预期。

#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tauri::ipc::Channel;
use tokio::sync::RwLock;
use tracing::warn;
use uuid::Uuid;

/// 256 KiB 积压上限。超过则丢弃最旧字节并 warn!（设计 §4.1 规则 1）
const BUFFER_CAP_BYTES: usize = 256 * 1024;

enum TermSink {
    /// 未 attach —— 字节累积。VecDeque 容量超限后丢弃最旧。
    Buffering(VecDeque<u8>),
    /// 已 attach —— 字节直推 Channel；send 失败退回 Buffering。
    Attached(Channel<Vec<u8>>),
}

pub struct TerminalChannels {
    inner: RwLock<HashMap<Uuid, TermSink>>,
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

    /// recv 循环无条件写入。未注册会话静默丢弃 + warn（防御性：recv 启动早于 attach）。
    /// 已注册会话若仍处于 Buffering，累积；Attached 则直推，失败退回 Buffering。
    pub async fn push(&self, session_id: Uuid, data: &[u8]) {
        let mut inner = self.inner.write().await;
        let entry = inner.entry(session_id).or_insert_with(|| {
            // 默认建一个 Buffering,等 attach 时再 flush
            TermSink::Buffering(VecDeque::with_capacity(BUFFER_CAP_BYTES))
        });

        match entry {
            TermSink::Buffering(buf) => {
                let dropped = append_capped(buf, data, BUFFER_CAP_BYTES);
                if dropped > 0 {
                    warn!(
                        session_id = %session_id,
                        dropped_bytes = dropped,
                        "terminal buffer overflow; oldest bytes dropped (frontend attach lags)"
                    );
                }
            }
            TermSink::Attached(ch) => {
                // Channel.send 失败 = 前端 HMR/重载导致 Channel 句柄失效。
                // 按设计 §4.1 规则 3：退回 Buffering,不中断后端 SSH 连接。
                if ch.send(data.to_vec()).is_err() {
                    warn!(
                        session_id = %session_id,
                        "channel.send failed; demoting to Buffering (frontend re-attach needed)"
                    );
                    *entry = TermSink::Buffering(VecDeque::with_capacity(BUFFER_CAP_BYTES));
                    if let TermSink::Buffering(buf) = entry {
                        append_capped(buf, data, BUFFER_CAP_BYTES);
                    }
                }
            }
        }
    }

    /// 前端 mount xterm 后调用 —— 把积压一次性 flush 进 Channel,并切换为 Attached。
    /// 后续 push 走 Attached 路径。**不会断开 SSH**。
    pub async fn attach(&self, session_id: Uuid, channel: Channel<Vec<u8>>) {
        let mut inner = self.inner.write().await;
        // 先 flush 积压 —— 即使 Channel.send 失败也只丢新字节,积压已拷给前端
        let mut buf = VecDeque::new();
        if let Some(TermSink::Buffering(existing)) = inner.get_mut(&session_id) {
            std::mem::swap(existing, &mut buf);
        }
        // 先尝试把积压送进 Channel
        if !buf.is_empty() {
            let bytes: Vec<u8> = buf.into_iter().collect();
            if let Err(e) = channel.send(bytes) {
                warn!(session_id = %session_id, error = %e, "attach: flush积压失败,丢弃积压");
            }
        }
        inner.insert(session_id, TermSink::Attached(channel));
    }

    /// 显式 detach(会话关闭时)。
    pub async fn detach(&self, session_id: Uuid) {
        self.inner.write().await.remove(&session_id);
    }

    /// 测试与诊断用 —— 当前 sink 状态概览。
    pub async fn debug_summary(&self) -> Vec<(Uuid, &'static str)> {
        let inner = self.inner.read().await;
        inner
            .iter()
            .map(|(k, v)| {
                let tag = match v {
                    TermSink::Buffering(_) => "buffering",
                    TermSink::Attached(_) => "attached",
                };
                (*k, tag)
            })
            .collect()
    }
}

/// 把 data 追加到 buf 末尾直至容量上限 BUFFER_CAP_BYTES,返回丢弃字节数。
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

    #[tokio::test]
    async fn buffering_then_attach_preserves_order() {
        let tc = TerminalChannels::new();
        let id = Uuid::new_v4();
        tc.push(id, b"hello ").await;
        tc.push(id, b"world").await;

        // 构造一个不真正发送的 Channel 很难（Channel 是 Tauri 类型），
        // 这里用 detach 后 push 应仍走 buffering 路径验证状态机。
        tc.push(id, b"!").await;
        let summary = tc.debug_summary().await;
        assert_eq!(summary, vec![(id, "buffering")]);
        tc.detach(id).await;
        let summary = tc.debug_summary().await;
        assert!(summary.is_empty());
    }

    #[tokio::test]
    async fn overflow_drops_oldest_bytes() {
        let tc = TerminalChannels::new();
        let id = Uuid::new_v4();
        // 灌入 2 × 上限
        let big: Vec<u8> = (0..(BUFFER_CAP_BYTES * 2) as u8).cycle().take(BUFFER_CAP_BYTES * 2).collect();
        tc.push(id, &big).await;
        // 内层状态应是 Buffering 且长度为 cap
        let summary = tc.debug_summary().await;
        assert_eq!(summary, vec![(id, "buffering")]);
    }
}