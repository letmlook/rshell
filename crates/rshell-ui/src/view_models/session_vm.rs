//! 会话 ViewModel
//!
//! 管理会话列表、选中状态和标签页。

use rshell_api::types::{ConnectionInfo, ConnectionState, SessionConfig};
use std::collections::HashMap;
use uuid::Uuid;

/// 标签页信息
#[derive(Debug, Clone)]
pub struct TabInfo {
    /// 会话 ID
    pub session_id: Uuid,
    /// 标签标题
    pub title: String,
    /// 是否已连接
    pub connected: bool,
}

/// 会话 ViewModel
pub struct SessionViewModel {
    /// 会话列表
    pub sessions: Vec<SessionConfig>,
    /// 当前选中会话
    pub selected_session: Option<Uuid>,
    /// 连接信息映射
    pub connection_info: HashMap<Uuid, ConnectionInfo>,
    /// 打开的标签页
    pub open_tabs: Vec<TabInfo>,
    /// 当前激活的标签索引
    pub active_tab: Option<usize>,
}

impl SessionViewModel {
    /// 创建新的 ViewModel
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            selected_session: None,
            connection_info: HashMap::new(),
            open_tabs: Vec::new(),
            active_tab: None,
        }
    }

    /// 处理事件（从后端接收）
    pub fn handle_event(&mut self, event: &rshell_api::AppEvent) {
        match event {
            rshell_api::AppEvent::SessionListChanged => {
                tracing::debug!("Session list changed");
            }
            rshell_api::AppEvent::SessionUpdated { session_id } => {
                tracing::debug!(session_id = %session_id, "Session updated");
                // 更新标签标题
                if let Some(tab) = self.open_tabs.iter_mut().find(|t| t.session_id == *session_id) {
                    if let Some(session) = self.sessions.iter().find(|s| s.id == *session_id) {
                        tab.title = session.name.clone();
                    }
                }
            }
            rshell_api::AppEvent::ConnectionStateChanged { session_id, state, info } => {
                if let Some(info) = info {
                    self.connection_info.insert(*session_id, info.clone());
                } else if *state == ConnectionState::Disconnected {
                    self.connection_info.remove(session_id);
                }
                // 更新标签连接状态
                if let Some(tab) = self.open_tabs.iter_mut().find(|t| t.session_id == *session_id) {
                    tab.connected = *state == ConnectionState::Connected;
                }
            }
            rshell_api::AppEvent::TerminalTitleChanged { session_id, title } => {
                // 更新标签标题
                if let Some(tab) = self.open_tabs.iter_mut().find(|t| t.session_id == *session_id) {
                    tab.title = title.clone();
                }
            }
            _ => {}
        }
    }

    /// 选择会话
    pub fn select_session(&mut self, session_id: Option<Uuid>) {
        self.selected_session = session_id;
    }

    /// 获取当前选中的会话
    pub fn get_selected_session(&self) -> Option<&SessionConfig> {
        self.selected_session
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
    }

    /// 获取会话的连接状态
    pub fn get_connection_state(&self, session_id: Uuid) -> ConnectionState {
        self.connection_info
            .get(&session_id)
            .map(|info| info.state)
            .unwrap_or(ConnectionState::Disconnected)
    }

    // ===== 标签管理 =====

    /// 打开新标签
    pub fn open_tab(&mut self, session_id: Uuid) {
        // 检查是否已打开
        if let Some(idx) = self.open_tabs.iter().position(|t| t.session_id == session_id) {
            self.active_tab = Some(idx);
            return;
        }

        let title = self.sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| format!("Session {}", &session_id.to_string()[..8]));

        let connected = self.get_connection_state(session_id) == ConnectionState::Connected;

        self.open_tabs.push(TabInfo {
            session_id,
            title,
            connected,
        });

        self.active_tab = Some(self.open_tabs.len() - 1);
    }

    /// 关闭标签
    pub fn close_tab(&mut self, index: usize) {
        if index < self.open_tabs.len() {
            self.open_tabs.remove(index);

            // 调整激活标签索引
            if let Some(active) = self.active_tab {
                if active == index {
                    // 关闭的是当前激活的标签
                    if self.open_tabs.is_empty() {
                        self.active_tab = None;
                    } else if index >= self.open_tabs.len() {
                        self.active_tab = Some(self.open_tabs.len() - 1);
                    } else {
                        self.active_tab = Some(index);
                    }
                } else if active > index {
                    self.active_tab = Some(active - 1);
                }
            }
        }
    }

    /// 切换标签
    pub fn switch_tab(&mut self, index: usize) {
        if index < self.open_tabs.len() {
            self.active_tab = Some(index);
            if let Some(tab) = self.open_tabs.get(index) {
                self.selected_session = Some(tab.session_id);
            }
        }
    }

    /// 获取当前激活的标签
    pub fn get_active_tab(&self) -> Option<&TabInfo> {
        self.active_tab.and_then(|idx| self.open_tabs.get(idx))
    }

    /// 获取当前激活标签的会话 ID
    pub fn get_active_session_id(&self) -> Option<Uuid> {
        self.get_active_tab().map(|tab| tab.session_id)
    }

    // ===== 命令生成 =====

    /// 创建会话命令
    pub fn create_session_command(&self, config: SessionConfig) -> rshell_api::AppCommand {
        rshell_api::AppCommand::CreateSession { config }
    }

    /// 更新会话命令
    pub fn update_session_command(&self, id: Uuid, config: SessionConfig) -> rshell_api::AppCommand {
        rshell_api::AppCommand::UpdateSession { id, config }
    }

    /// 删除会话命令
    pub fn delete_session_command(&self, id: Uuid) -> rshell_api::AppCommand {
        rshell_api::AppCommand::DeleteSession { id }
    }

    /// 连接会话命令
    pub fn connect_session_command(&self, session_id: Uuid) -> rshell_api::AppCommand {
        rshell_api::AppCommand::ConnectSession { session_id }
    }

    /// 断开会话命令
    pub fn disconnect_session_command(&self, session_id: Uuid) -> rshell_api::AppCommand {
        rshell_api::AppCommand::DisconnectSession { session_id }
    }

    // ===== 数据同步 =====

    /// 添加会话（从后端同步）
    pub fn set_sessions(&mut self, sessions: Vec<SessionConfig>) {
        self.sessions = sessions;
    }

    /// 添加单个会话
    pub fn add_session(&mut self, config: SessionConfig) {
        if !self.sessions.iter().any(|s| s.id == config.id) {
            self.sessions.push(config);
        }
    }

    /// 移除会话
    pub fn remove_session(&mut self, id: Uuid) {
        self.sessions.retain(|s| s.id != id);
        if self.selected_session == Some(id) {
            self.selected_session = None;
        }
        self.connection_info.remove(&id);

        // 关闭相关标签
        if let Some(idx) = self.open_tabs.iter().position(|t| t.session_id == id) {
            self.close_tab(idx);
        }
    }
}
