//! 终端服务 —— 切片 2.1 瘦身（设计 §2.2）
//!
//! 原实现持 alacritty_terminal 缓冲,由 §2 D1 决策删除 —— xterm.js 接管渲染后,
//! 后端只剩"记录每会话尺寸"这一职责。TerminalBufferSnapshot / process_output /
//! get_buffer_snapshot / send_input 已全部移除;recv 循环直接把字节喂
//! `TerminalChannels`(见 `src-tauri/src/terminal.rs`)→ 前端 xterm。

use crate::error::CoreError;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, instrument};
use uuid::Uuid;

/// 输出泵的信道元素：`(session_id, terminal_id, data)`。
///
/// `terminal_id` 必须带上：一条连接可以有多个 pty（多个标签会话），
/// 壳层按它把字节投到对应的前端通道。
type OutputSender = tokio::sync::mpsc::UnboundedSender<(Uuid, Uuid, Vec<u8>)>;

/// 终端服务 —— 仅持"已知会话 + 最近一次报告尺寸",无 alacritty 状态机。
///
/// 切片 2.1 落地：原 328 行 → 当前 ~60 行。`resize` 由 `dispatcher` 接收前端
/// `ResizeTerminal` 命令后调用,用于将来 PTY 透传或转发给对端 SSH shell 的
/// window-change 请求(切片 4+ 实现)。
pub struct TerminalService {
    /// 已知会话 id 与最近一次报告尺寸
    sizes: Arc<RwLock<HashMap<Uuid, (u16, u16)>>>,
    output_sender: Arc<RwLock<Option<OutputSender>>>,
}

impl TerminalService {
    pub fn new(_event_bus: Arc<crate::event_bus::EventBus>) -> Self {
        Self {
            sizes: Arc::new(RwLock::new(HashMap::new())),
            output_sender: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_output_sender(&self, sender: OutputSender) {
        *self
            .output_sender
            .write()
            .expect("output sender lock poisoned") = Some(sender);
    }

    pub fn push_output(
        &self,
        session_id: Uuid,
        terminal_id: Uuid,
        data: Vec<u8>,
    ) -> Result<(), CoreError> {
        let sink = self
            .output_sender
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        let sender = sink.as_ref().ok_or_else(|| {
            CoreError::InvalidState("Terminal output sink is not configured".into())
        })?;
        sender
            .send((session_id, terminal_id, data))
            .map_err(|_| CoreError::InvalidState("Terminal output sink is closed".into()))
    }

    /// 登记会话终端;启动时由 setup 或首条 recv 字节触发。
    #[instrument(skip(self))]
    pub fn create_terminal(&self, id: Uuid, cols: u16, rows: u16) -> Result<(), CoreError> {
        let mut sizes = self
            .sizes
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        sizes.insert(id, (cols, rows));
        debug!(terminal_id = %id, cols, rows, "Terminal registered");
        Ok(())
    }

    /// 会话关闭时清理。
    #[instrument(skip(self))]
    pub fn destroy_terminal(&self, id: Uuid) -> Result<(), CoreError> {
        let mut sizes = self
            .sizes
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        sizes.remove(&id);
        debug!(terminal_id = %id, "Terminal removed");
        Ok(())
    }

    /// 前端权威 resize（设计 §4.2 "终端尺寸"行）。
    #[instrument(skip(self))]
    pub fn resize(&self, terminal_id: Uuid, cols: u16, rows: u16) -> Result<(), CoreError> {
        let mut sizes = self
            .sizes
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        sizes.insert(terminal_id, (cols, rows));
        debug!(terminal_id = %terminal_id, cols, rows, "Terminal resized");
        Ok(())
    }

    pub fn size(&self, terminal_id: Uuid) -> Option<(u16, u16)> {
        self.sizes.read().ok()?.get(&terminal_id).copied()
    }

    /// 诊断用：当前已知会话尺寸快照。
    #[allow(dead_code)]
    pub fn snapshot(&self) -> Vec<(Uuid, u16, u16)> {
        let sizes = self.sizes.read().expect("sizes lock poisoned");
        sizes.iter().map(|(id, (c, r))| (*id, *c, *r)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::EventBus;

    #[test]
    fn create_resize_destroy_roundtrip() {
        let bus = Arc::new(EventBus::new());
        let svc = TerminalService::new(bus);
        let id = Uuid::new_v4();

        svc.create_terminal(id, 80, 24).unwrap();
        svc.resize(id, 120, 30).unwrap();
        let snap = svc.snapshot();
        assert_eq!(snap, vec![(id, 120, 30)]);

        svc.destroy_terminal(id).unwrap();
        assert!(svc.snapshot().is_empty());
    }

    #[test]
    fn output_sender_routes_bytes_without_reencoding() {
        let service = TerminalService::new(Arc::new(EventBus::new()));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        service.set_output_sender(tx);
        let session_id = Uuid::new_v4();
        let terminal_id = Uuid::new_v4();
        service
            .push_output(session_id, terminal_id, vec![0, 0xff, b'\n'])
            .unwrap();
        // 输出带 terminal_id：壳层据此把字节投到对应标签的 pty
        assert_eq!(
            rx.try_recv().unwrap(),
            (session_id, terminal_id, vec![0, 0xff, b'\n'])
        );
    }
}
