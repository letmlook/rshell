//! 终端 ViewModel
//!
//! 后端状态在终端视图的投影，包含后端状态投影 + 本地 UI 状态。

use rshell_api::types::{ConnectionState, TerminalBufferSnapshot};
use uuid::Uuid;

/// 终端 ViewModel - 后端状态在终端视图的投影
pub struct TerminalViewModel {
    /// 会话 ID
    pub session_id: Uuid,
    /// 连接状态
    pub connection_state: ConnectionState,
    /// 终端标题
    pub title: String,
    /// 终端缓冲区快照
    pub buffer: Option<TerminalBufferSnapshot>,
    /// 滚动偏移（本地 UI 状态）
    pub scroll_offset: usize,
    /// 是否搜索模式（本地 UI 状态）
    pub is_search_mode: bool,
    /// 搜索查询（本地 UI 状态）
    pub search_query: String,
}

impl TerminalViewModel {
    /// 创建新的 ViewModel
    pub fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            connection_state: ConnectionState::Disconnected,
            title: String::new(),
            buffer: None,
            scroll_offset: 0,
            is_search_mode: false,
            search_query: String::new(),
        }
    }

    /// 处理事件（从后端接收）
    pub fn handle_event(&mut self, event: &rshell_api::AppEvent) {
        match event {
            rshell_api::AppEvent::ConnectionStateChanged { session_id, state, info: _ } => {
                if *session_id == self.session_id {
                    self.connection_state = *state;
                }
            }
            rshell_api::AppEvent::TerminalOutput { session_id, data } => {
                if *session_id == self.session_id {
                    // raw 字节流只用于日志/触发器匹配，不再驱动渲染。
                    // 真正的渲染由 TerminalBufferUpdated 驱动（见下）。
                    tracing::trace!(
                        session_id = %session_id,
                        bytes = data.len(),
                        "Received terminal output (raw bytes)"
                    );
                }
            }
            rshell_api::AppEvent::TerminalBufferUpdated { session_id, snapshot } => {
                if *session_id == self.session_id {
                    // 后端已解析 VT 序列并生成完整 snapshot，
                    // 直接喂给本地 buffer，触发 Render 重绘。
                    self.update_buffer(snapshot.clone());
                }
            }
            rshell_api::AppEvent::TerminalTitleChanged { session_id, title } => {
                if *session_id == self.session_id {
                    self.title = title.clone();
                }
            }
            rshell_api::AppEvent::SessionUpdated { session_id }
                if *session_id == self.session_id => {
                    // 会话配置更新，可触发视图刷新
                    tracing::debug!(session_id = %session_id, "Session updated");
                }
            _ => {}
        }
    }

    /// 用户输入（键盘输入）
    pub fn on_user_input(&self, data: &[u8]) -> rshell_api::AppCommand {
        rshell_api::AppCommand::SendInput {
            session_id: self.session_id,
            data: data.to_vec(),
        }
    }

    /// 用户调整终端大小
    pub fn on_resize(&self, cols: u16, rows: u16) -> rshell_api::AppCommand {
        rshell_api::AppCommand::ResizeTerminal {
            session_id: self.session_id,
            cols,
            rows,
        }
    }

    /// 更新缓冲区快照
    pub fn update_buffer(&mut self, snapshot: TerminalBufferSnapshot) {
        self.title = snapshot.title.clone();
        self.buffer = Some(snapshot);
    }

    /// 切换搜索模式
    pub fn toggle_search_mode(&mut self) {
        self.is_search_mode = !self.is_search_mode;
        if !self.is_search_mode {
            self.search_query.clear();
        }
    }

    /// 更新搜索查询
    pub fn update_search_query(&mut self, query: String) {
        self.search_query = query;
    }

    /// 滚动
    pub fn scroll(&mut self, delta: i32) {
        if delta > 0 {
            self.scroll_offset = self.scroll_offset.saturating_add(delta as usize);
        } else {
            self.scroll_offset = self.scroll_offset.saturating_sub((-delta) as usize);
        }
    }
}
