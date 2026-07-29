//! 应用根组件
//!
//! RShell 主窗口视图，包含会话树、终端标签页、文件管理器和传输队列。

#![allow(dead_code)]

use gpui::{div, prelude::*, px, rgb, Window};
use rshell_api::{AppCommand, AppEvent};
use rshell_api::types::{ConnectionState, SessionConfig};
use uuid::Uuid;

use crate::bridge::AppBridge;
use crate::views::file_manager_view::FileManagerView;
use crate::views::key_management_view::KeyManagementView;
use crate::views::plugin_manager_view::PluginManagerView;
use crate::views::quick_commands_view::QuickCommandsView;
use crate::views::session_view::SessionView;
use crate::views::terminal_view::TerminalView;
use crate::views::theme_settings_view::ThemeSettingsView;
use crate::views::transfer_view::TransferView;
use crate::views::tunnel_panel_view::TunnelPanelView;
use crate::views::compose_pane_view::ComposePaneView;

/// 应用根组件
pub struct RshellApp {
    /// 后端桥接
    bridge: AppBridge,
    /// 当前激活的标签索引
    active_tab: Option<usize>,
    /// 标签页列表
    tabs: Vec<TabInfo>,
    /// 是否显示文件管理器
    show_file_manager: bool,
    /// 当前激活的 Dock 面板
    active_panel: PanelKind,

    /// 已挂载的视图（10 个）
    file_manager: gpui::Entity<FileManagerView>,
    session_view: gpui::Entity<SessionView>,
    terminal_view: gpui::Entity<TerminalView>,
    transfer_view: gpui::Entity<TransferView>,
    key_mgmt_view: gpui::Entity<KeyManagementView>,
    theme_view: gpui::Entity<ThemeSettingsView>,
    quick_cmds_view: gpui::Entity<QuickCommandsView>,
    compose_view: gpui::Entity<ComposePaneView>,
    tunnel_view: gpui::Entity<TunnelPanelView>,
    plugin_view: gpui::Entity<PluginManagerView>,

    /// 会话列表（从后端同步）
    sessions: Vec<SessionInfo>,
}

/// 当前激活的中央面板
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Terminal,
    FileManager,
    Keys,
    Theme,
    QuickCommands,
    Compose,
    Tunnels,
    Plugins,
}

/// 标签信息
struct TabInfo {
    title: String,
    connected: bool,
    session_id: Uuid,
}

/// 会话信息（UI 用）
struct SessionInfo {
    id: Uuid,
    name: String,
    state: ConnectionState,
}

impl RshellApp {
    /// 创建新的应用实例
    pub fn new(bridge: AppBridge, cx: &mut gpui::Context<Self>) -> Self {
        let file_manager = cx.new(|cx| FileManagerView::new(cx));
        let session_view = cx.new(|cx| SessionView::new(cx));
        let terminal_view = cx.new(|_cx| TerminalView::new());
        let transfer_view = cx.new(|cx| TransferView::new(cx));
        let key_mgmt_view = cx.new(|cx| KeyManagementView::new(cx));
        let theme_view = cx.new(|cx| ThemeSettingsView::new(cx));
        let quick_cmds_view = cx.new(|cx| QuickCommandsView::new(cx));
        let compose_view = cx.new(|cx| ComposePaneView::new(cx));
        let tunnel_view = cx.new(|cx| TunnelPanelView::new(cx));
        let plugin_view = cx.new(|cx| PluginManagerView::new(cx));

        Self {
            bridge,
            active_tab: None,
            tabs: Vec::new(),
            show_file_manager: false,
            active_panel: PanelKind::Terminal,
            file_manager,
            session_view,
            terminal_view,
            transfer_view,
            key_mgmt_view,
            theme_view,
            quick_cmds_view,
            compose_view,
            tunnel_view,
            plugin_view,
            sessions: Vec::new(),
        }
    }

    /// 切换文件管理器显示状态
    pub fn toggle_file_manager(&mut self) {
        self.show_file_manager = !self.show_file_manager;
    }

    /// 处理后端事件（在 render 前调用）
    fn process_events(&mut self) {
        let events = self.bridge.drain_events();
        for event in events {
            match event {
                AppEvent::SessionListChanged => {
                    // 会话列表变化，可以触发重新加载
                    tracing::debug!("Session list changed");
                }
                AppEvent::ConnectionStateChanged { session_id, state, .. } => {
                    // 更新会话状态
                    for session in &mut self.sessions {
                        if session.id == session_id {
                            session.state = state;
                        }
                    }
                    // 更新标签页连接状态
                    for tab in &mut self.tabs {
                        if tab.session_id == session_id {
                            tab.connected = state == ConnectionState::Connected;
                        }
                    }
                }
                AppEvent::TerminalTitleChanged { session_id, title } => {
                    for tab in &mut self.tabs {
                        if tab.session_id == session_id {
                            tab.title = title.clone();
                        }
                    }
                }
                AppEvent::TransferTaskAdded { task_id, filename, .. } => {
                    tracing::info!("Transfer task added: {} ({})", filename, task_id);
                }
                AppEvent::TransferCompleted { task_id } => {
                    tracing::info!("Transfer completed: {}", task_id);
                }
                AppEvent::TransferFailed { task_id, error } => {
                    tracing::error!("Transfer failed: {} - {}", task_id, error);
                }
                _ => {}
            }
        }
    }

    /// 发送命令到后端
    fn send_command(&self, command: AppCommand) {
        self.bridge.send_command(command);
    }

    /// 连接会话
    fn connect_session(&self, session_id: Uuid) {
        self.send_command(AppCommand::ConnectSession { session_id });
    }

    /// 断开会话
    fn disconnect_session(&self, session_id: Uuid) {
        self.send_command(AppCommand::DisconnectSession { session_id });
    }

    /// 创建示例会话（用于测试）
    fn create_demo_session(&mut self) {
        let config = SessionConfig {
            id: Uuid::new_v4(),
            name: format!("Session {}", self.sessions.len() + 1),
            folder_id: None,
            host: "localhost".to_string(),
            port: 22,
            protocol: rshell_api::types::Protocol::SSH,
            auth_method: rshell_api::types::AuthMethod::Password {
                username: "user".to_string(),
                password: String::new(),
            },
        };

        self.sessions.push(SessionInfo {
            id: config.id,
            name: config.name.clone(),
            state: ConnectionState::Disconnected,
        });

        self.send_command(AppCommand::CreateSession { config });
    }
}

impl Render for RshellApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        // 处理后端事件
        self.process_events();

        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .flex()
            .flex_row()
            .child(
                // 左侧：会话树（使用真实 SessionView 组件）
                div()
                    .w(px(250.0))
                    .bg(rgb(0x252535))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .child(self.session_view.clone())
                    .child(
                        // 新建会话按钮
                        div()
                            .mt_2()
                            .p_2()
                            .child(
                                div()
                                    .bg(rgb(0x3b82f6))
                                    .px(px(12.0))
                                    .py(px(6.0))
                                    .rounded(px(4.0))
                                    .text_color(rgb(0xffffff))
                                    .text_sm()
                                    .text_center()
                                    .child("+ 新建会话"),
                            ),
                    )
                    .child(
                        div()
                            .p_2()
                            .child(
                                div()
                                    .bg(rgb(0x6366f1))
                                    .px(px(12.0))
                                    .py(px(6.0))
                                    .rounded(px(4.0))
                                    .text_color(rgb(0xffffff))
                                    .text_sm()
                                    .text_center()
                                    .child(if self.show_file_manager { "隐藏文件管理器" } else { "显示文件管理器" }),
                            ),
                    ),
            )
            .child(
                // 中心：当前激活面板
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        // 标签栏（保留 — 多会话标签是核心 UX）
                        div()
                            .h(px(32.0))
                            .bg(rgb(0x2d2d3d))
                            .flex()
                            .items_center()
                            .px_2()
                            .child(
                                if self.tabs.is_empty() {
                                    div()
                                        .text_color(rgb(0x888888))
                                        .child("无打开的标签页")
                                        .into_any()
                                } else {
                                    let mut tabs_div = div().flex().flex_row().gap_1();
                                    for (idx, tab) in self.tabs.iter().enumerate() {
                                        let is_active = self.active_tab == Some(idx);
                                        let bg_color = if is_active { rgb(0x3d3d5d) } else { rgb(0x2d2d3d) };
                                        let status_color = if tab.connected { rgb(0x00ff00) } else { rgb(0x888888) };

                                        tabs_div = tabs_div.child(
                                            div()
                                                .px_2()
                                                .py_1()
                                                .bg(bg_color)
                                                .rounded(px(4.0))
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .w(px(8.0))
                                                        .h(px(8.0))
                                                        .rounded_full()
                                                        .bg(status_color),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(rgb(0xffffff))
                                                        .child(tab.title.clone()),
                                                ),
                                        );
                                    }
                                    tabs_div.into_any()
                                },
                            ),
                    )
                    .child(
                        // 当前面板内容（按 PanelKind 路由）
                        div()
                            .flex_1()
                            .child(self.render_active_panel()),
                    ),
            )
            .child(
                // 底部：传输队列（使用真实 TransferView 组件）
                div()
                    .h(px(150.0))
                    .bg(rgb(0x252535))
                    .border_t_1()
                    .border_color(rgb(0x3d3d4d))
                    .child(self.transfer_view.clone()),
            )
    }
}

impl RshellApp {
    /// 渲染会话列表
    fn render_session_list(&self) -> impl IntoElement {
        let mut list = div().mt_2().flex().flex_col().gap_1();

        if self.sessions.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .text_color(rgb(0x666666))
                    .text_sm()
                    .child("暂无会话"),
            );
        } else {
            for session in &self.sessions {
                let state_color = match session.state {
                    ConnectionState::Connected => rgb(0x00ff00),
                    ConnectionState::Connecting | ConnectionState::Authenticating => rgb(0xffff00),
                    ConnectionState::Disconnected => rgb(0x888888),
                    ConnectionState::Disconnecting => rgb(0xff8800),
                };

                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .bg(rgb(0x2d2d3d))
                        .rounded(px(4.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(8.0))
                                .h(px(8.0))
                                .rounded_full()
                                .bg(state_color),
                        )
                        .child(
                            div()
                                .text_color(rgb(0xffffff))
                                .text_sm()
                                .child(session.name.clone()),
                        ),
                );
            }
        }

        list
    }

    /// 按当前 PanelKind 路由渲染中央面板
    fn render_active_panel(&self) -> gpui::AnyElement {
        match self.active_panel {
            PanelKind::Terminal => {
                if self.show_file_manager {
                    // 兼容旧逻辑：show_file_manager 覆盖 PanelKind::FileManager
                    div()
                        .size_full()
                        .child(self.file_manager.clone())
                        .into_any()
                } else if self.tabs.is_empty() {
                    div()
                        .size_full()
                        .bg(rgb(0x000000))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_color(rgb(0x888888))
                                .child("从左侧选择会话并连接以开始"),
                        )
                        .into_any()
                } else {
                    div()
                        .size_full()
                        .child(self.terminal_view.clone())
                        .into_any()
                }
            }
            PanelKind::FileManager => div().size_full().child(self.file_manager.clone()).into_any(),
            PanelKind::Keys => div().size_full().child(self.key_mgmt_view.clone()).into_any(),
            PanelKind::Theme => div().size_full().child(self.theme_view.clone()).into_any(),
            PanelKind::QuickCommands => div().size_full().child(self.quick_cmds_view.clone()).into_any(),
            PanelKind::Compose => div().size_full().child(self.compose_view.clone()).into_any(),
            PanelKind::Tunnels => div().size_full().child(self.tunnel_view.clone()).into_any(),
            PanelKind::Plugins => div().size_full().child(self.plugin_view.clone()).into_any(),
        }
    }
}
