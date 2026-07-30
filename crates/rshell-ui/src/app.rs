//! 应用根组件
//!
//! RShell 主窗口视图，包含会话树、终端标签页、文件管理器和传输队列。

#![allow(dead_code)]

use gpui::{div, prelude::*, px, rgb, Window};
use rshell_api::types::{ConnectionState, SessionConfig};
use rshell_api::{AppCommand, AppEvent};
use uuid::Uuid;

use crate::bridge::AppBridge;
use crate::view_models::session_vm::SessionViewModel;
use crate::view_models::terminal_vm::TerminalViewModel;
use crate::view_models::transfer_vm::TransferViewModel;
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

    /// 已挂载的 ViewModel（3 个核心）
    session_vm: gpui::Entity<SessionViewModel>,
    terminal_vm: gpui::Entity<TerminalViewModel>,
    transfer_vm: gpui::Entity<TransferViewModel>,
    /// 当前激活 tab 的 session id（用于 TerminalBufferUpdated 路由）
    active_session_id: Uuid,

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

    /// 会话列表（rshell_api::types::SessionInfo, 与 SessionView 共享, 状态在 handle_event 增量更新）
    sessions: Vec<rshell_api::types::SessionInfo>,

    /// 待显示的主机密钥信任对话框 (None = 不显示)
    pending_host_key_prompt: Option<HostKeyPromptData>,
}

/// 待显示的主机密钥信任对话框数据
#[derive(Clone)]
struct HostKeyPromptData {
    decision_id: Uuid,
    host: String,
    port: u16,
    key_type: String,
    fingerprint: String,
    public_key_blob: String,
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

impl RshellApp {
    /// 创建新的应用实例
    pub fn new(bridge: AppBridge, cx: &mut gpui::Context<Self>) -> Self {
        let session_vm = cx.new(|_cx| SessionViewModel::new());
        let terminal_vm = cx.new(|_cx| TerminalViewModel::new(Uuid::new_v4()));
        let transfer_vm = cx.new(|_cx| TransferViewModel::new());

        let file_manager = cx.new(FileManagerView::new);
        let session_view = cx.new(SessionView::new);
        let terminal_view = cx.new(TerminalView::new);
        let transfer_view = cx.new(TransferView::new);
        let key_mgmt_view = cx.new(KeyManagementView::new);
        let theme_view = cx.new(ThemeSettingsView::new);
        let quick_cmds_view = cx.new(QuickCommandsView::new);
        let compose_view = cx.new(ComposePaneView::new);
        let tunnel_view = cx.new(TunnelPanelView::new);
        let plugin_view = cx.new(PluginManagerView::new);

        let app = Self {
            bridge,
            active_tab: None,
            tabs: Vec::new(),
            show_file_manager: false,
            active_panel: PanelKind::Terminal,
            session_vm,
            terminal_vm,
            transfer_vm,
            active_session_id: Uuid::nil(),
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
            pending_host_key_prompt: None,
        };
        // 启动时拉所有列表 (后端立即 publish XSnapshot 事件)
        app.refresh_all_lists();
        app
    }

    /// 拉所有后端列表 (XSnapshot 事件)
    fn refresh_all_lists(&self) {
        self.send_command(AppCommand::ListSessions);
        self.send_command(AppCommand::ListTunnels);
        self.send_command(AppCommand::ListKeys);
        self.send_command(AppCommand::ListPlugins);
        self.send_command(AppCommand::ListThemes);
    }

    /// 切换文件管理器显示状态
    pub fn toggle_file_manager(&mut self) {
        self.show_file_manager = !self.show_file_manager;
    }

    /// 处理后端事件（在 render 前调用）
    fn process_events(&mut self, cx: &mut gpui::Context<Self>) {
        let events = self.bridge.drain_events();
        for event in events {
            // 1) 路由到各 ViewModel（让 VM 拥有完整状态）
            self.session_vm.update(cx, |vm, _| vm.handle_event(&event));
            self.terminal_vm.update(cx, |vm, _| vm.handle_event(&event));
            self.transfer_vm.update(cx, |vm, _| vm.handle_event(&event));

            // 2) TerminalBufferUpdated → 转给 TerminalView 让它渲染实际 buffer
            if let rshell_api::AppEvent::TerminalBufferUpdated { session_id, snapshot } = &event {
                if *session_id == self.active_session_id {
                    self.terminal_view.update(cx, |v, _| v.update_buffer(snapshot.clone()));
                }
            }

            // 3) 快照事件 → 推到对应 view 的 update_*()
            // ListTriggers / ListQuickCommands / ListTunnels 走回原 ListChanged
            // 事件 (因为 ListTriggers/ListQuickCommands 没有自己的 snapshot event)
            match &event {
                AppEvent::SessionsSnapshot { sessions } => {
                    // 把 Vec<SessionConfig> 转成 Vec<SessionInfo> 给 view
                    // view 内部在 handle_event 增量更新 state, 初次填 Disconnected
                    let infos: Vec<rshell_api::types::SessionInfo> = sessions
                        .iter()
                        .map(|cfg: &SessionConfig| rshell_api::types::SessionInfo {
                            id: cfg.id,
                            config: cfg.clone(),
                            state: ConnectionState::Disconnected,
                        })
                        .collect();
                    self.sessions = infos.clone();
                    self.session_view.update(cx, |v, _| v.update_sessions(infos));
                }
                AppEvent::TunnelsSnapshot { tunnels } => {
                    let t = tunnels.clone();
                    self.tunnel_view.update(cx, |v, _| v.update_tunnels(t));
                }
                AppEvent::KeysSnapshot { keys } => {
                    let k = keys.clone();
                    self.key_mgmt_view.update(cx, |v, _| v.update_keys(k));
                }
                AppEvent::PluginsSnapshot { plugins } => {
                    let p = plugins.clone();
                    self.plugin_view.update(cx, |v, _| v.update_plugins(p));
                }
                AppEvent::ThemesSnapshot {
                    current_theme,
                    current_scheme,
                    available_themes,
                    available_schemes,
                } => {
                    let t = current_theme.clone();
                    let s = current_scheme.clone();
                    let at = available_themes.clone();
                    let ac = available_schemes.clone();
                    self.theme_view.update(cx, |v, _| v.update_themes(t, s, at, ac));
                }
                AppEvent::SessionListChanged => {
                    self.send_command(AppCommand::ListSessions);
                }
                AppEvent::ActiveTunnelsChanged => {
                    self.send_command(AppCommand::ListTunnels);
                }
                _ => {}
            }

            // 3) 本地状态同步（tabs/sessions UI 状态）
            match &event {
                AppEvent::ConnectionStateChanged { session_id, state, .. } => {
                    // 更新本地会话状态
                    for session in &mut self.sessions {
                        if session.id == *session_id {
                            session.state = *state;
                        }
                    }
                    // 更新标签页连接状态
                    for tab in &mut self.tabs {
                        if tab.session_id == *session_id {
                            tab.connected = *state == ConnectionState::Connected;
                        }
                    }
                    // 自动打开 tab（如果连接成功且未在列表中）
                    if *state == ConnectionState::Connected {
                        self.open_tab_for_session(cx, *session_id);
                    }
                }
                AppEvent::TerminalBufferUpdated { session_id, .. } => {
                    // 确保 TerminalInputState global 指向正确的 session
                    self.update_active_session(cx, *session_id);
                }
                AppEvent::TerminalTitleChanged { session_id, title } => {
                    for tab in &mut self.tabs {
                        if tab.session_id == *session_id {
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
                AppEvent::HostKeyMismatch {
                    decision_id,
                    host,
                    port,
                    key_type,
                    expected: _,
                    received,
                    public_key_blob,
                } if !decision_id.is_nil() => {
                    self.pending_host_key_prompt = Some(HostKeyPromptData {
                        decision_id: *decision_id,
                        host: host.clone(),
                        port: *port,
                        key_type: key_type.clone(),
                        fingerprint: received.clone(),
                        public_key_blob: public_key_blob.clone(),
                    });
                }
                AppEvent::ClipboardCopy { text } => {
                    // 后端请求拷贝文本到系统剪贴板。
                    // arboard 在 Windows 上走 Ole32 / OLE 自动化的 Clipboard API,
                    // 在 macOS 上走 NSPasteboard, Linux 上走 X11 / wayland-clipboard。
                    match arboard::Clipboard::new() {
                        Ok(mut cb) => match cb.set_text(text.clone()) {
                            Ok(()) => tracing::info!(
                                bytes = text.len(),
                                "ClipboardCopy → 系统剪贴板写入成功"
                            ),
                            Err(e) => tracing::error!(
                                bytes = text.len(),
                                error = %e,
                                "ClipboardCopy: arboard.set_text 失败"
                            ),
                        },
                        Err(e) => tracing::error!(
                            bytes = text.len(),
                            error = %e,
                            "ClipboardCopy: 无法获取 arboard::Clipboard 句柄"
                        ),
                    }
                }
                _ => {}
            }
        }
    }

    /// 连接成功时自动打开对应 session 的 tab
    fn open_tab_for_session(&mut self, cx: &mut gpui::Context<Self>, session_id: Uuid) {
        if self.tabs.iter().any(|t| t.session_id == session_id) {
            // 已存在则切换到该 tab
            if let Some(idx) = self.tabs.iter().position(|t| t.session_id == session_id) {
                self.active_tab = Some(idx);
                self.update_active_session(cx, session_id);
            }
            return;
        }
        // 新建 tab
        let title = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.config.name.clone())
            .unwrap_or_else(|| format!("Session {}", &session_id.to_string()[..8]));
        self.tabs.push(TabInfo {
            title,
            connected: true,
            session_id,
        });
        self.active_tab = Some(self.tabs.len() - 1);
        self.update_active_session(cx, session_id);
    }

    /// 切换激活 tab 时同步更新 TerminalInputState global（用于 key listener）
    fn update_active_session(&mut self, cx: &mut gpui::Context<Self>, session_id: Uuid) {
        self.active_session_id = session_id;
        cx.set_global(crate::views::terminal_view::TerminalInputState { session_id });
        // 同时设置 TerminalView 的 session_id 字段（即使 listener 不用，也可用于 future 内部逻辑）
        self.terminal_view.update(cx, |v, _| v.set_session_id(session_id));
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

    /// 关闭 tab: 断开会话 + 从 tabs 移除
    fn close_tab(&mut self, cx: &mut gpui::Context<Self>, session_id: Uuid) {
        self.send_command(AppCommand::DisconnectSession { session_id });
        self.tabs.retain(|t| t.session_id != session_id);
        if self.tabs.is_empty() {
            self.active_tab = None;
            self.update_active_session(cx, Uuid::nil());
        } else if let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.session_id == self.active_session_id)
        {
            self.active_tab = Some(idx);
        } else {
            self.active_tab = Some(0);
            self.update_active_session(cx, self.tabs[0].session_id);
        }
        cx.notify();
    }

    /// 创建示例会话（用于测试 + 不实现 dialog 前的快速创建入口）
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

        self.sessions.push(rshell_api::types::SessionInfo {
            id: config.id,
            config: config.clone(),
            state: ConnectionState::Disconnected,
        });

        self.send_command(AppCommand::CreateSession { config });
    }
}

impl Render for RshellApp {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        // 处理后端事件（路由到 ViewModel）
        self.process_events(cx);

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
                                    .id("new-session-btn")
                                    .bg(rgb(0x3b82f6))
                                    .px(px(12.0))
                                    .py(px(6.0))
                                    .rounded(px(4.0))
                                    .text_color(rgb(0xffffff))
                                    .text_sm()
                                    .text_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x2563eb)))
                                    .child("+ 新建会话")
                                    .on_click(cx.listener(|this, _, _window, _cx| {
                                        // 无 dialog 阶段: 直接创建 localhost 占位 session
                                        this.create_demo_session();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .p_2()
                            .child(
                                div()
                                    .id("toggle-file-manager-btn")
                                    .bg(rgb(0x6366f1))
                                    .px(px(12.0))
                                    .py(px(6.0))
                                    .rounded(px(4.0))
                                    .text_color(rgb(0xffffff))
                                    .text_sm()
                                    .text_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x4f46e5)))
                                    .child(if self.show_file_manager {
                                        "隐藏文件管理器"
                                    } else {
                                        "显示文件管理器"
                                    })
                                    .on_click(cx.listener(|this, _, _window, _cx| {
                                        this.toggle_file_manager();
                                    })),
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
                                    gpui::IntoElement::into_any_element(
                                        div()
                                            .text_color(rgb(0x888888))
                                            .child("无打开的标签页"),
                                    )
                                } else {
                                    let mut tabs: Vec<gpui::AnyElement> = Vec::with_capacity(self.tabs.len());
                                    // 同样的 take/replace 模式
                                    let tabs_data = std::mem::take(&mut self.tabs);
                                    for (idx, tab) in tabs_data.iter().enumerate() {
                                        let is_active = self.active_tab == Some(idx);
                                        let bg_color = if is_active { rgb(0x3d3d5d) } else { rgb(0x2d2d3d) };
                                        let status_color = if tab.connected { rgb(0x00ff00) } else { rgb(0x888888) };
                                        let title = tab.title.clone();
                                        let session_id = tab.session_id;
                                        let close_id = tab.session_id;

                                        tabs.push(gpui::IntoElement::into_any_element(
                                            div()
                                                .id(("tab", idx))
                                                .px_2()
                                                .py_1()
                                                .bg(bg_color)
                                                .rounded(px(4.0))
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .cursor_pointer()
                                                .hover(|s| s.bg(rgb(0x4d4d6d)))
                                                .on_click(cx.listener(move |this, _, _window, cx| {
                                                    this.active_tab = Some(idx);
                                                    this.update_active_session(cx, session_id);
                                                    cx.notify();
                                                }))
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
                                                        .text_sm()
                                                        .child(title),
                                                )
                                                .child(
                                                    div()
                                                        .id(("tab-close", idx))
                                                        .ml_1()
                                                        .text_color(rgb(0x888888))
                                                        .text_sm()
                                                        .cursor_pointer()
                                                        .hover(|s| s.text_color(rgb(0xff6666)))
                                                        .child("×")
                                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                                            this.close_tab(cx, close_id);
                                                        })),
                                                ),
                                        ));
                                    }
                                    let _ = std::mem::replace(&mut self.tabs, tabs_data);
                                    gpui::IntoElement::into_any_element(
                                        div().flex().flex_row().gap_1().children(tabs),
                                    )
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
            .children(self.render_host_key_dialog_overlay(cx))
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
                                .child(session.config.name.clone()),
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

    /// 渲染主机密钥信任对话框 modal overlay (None 时返回空 Vec)
    fn render_host_key_dialog_overlay(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let Some(p) = self.pending_host_key_prompt.clone() else {
            return vec![];
        };
        let decision_id = p.decision_id;

        let overlay = div()
            .absolute()
            .inset_0()
            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.7))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id(("hostkey-modal", 0usize))
                    .w(px(540.0))
                    .bg(rgb(0x1e1e2e))
                    .border_1()
                    .border_color(rgb(0x45475a))
                    .rounded(px(8.0))
                    .p(px(20.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .text_color(rgb(0xcdd6f4))
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("信任主机密钥"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xbac2de))
                            .text_sm()
                            .child(format!("主机: {}:{}", p.host, p.port)),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xbac2de))
                            .text_xs()
                            .child(format!("类型: {}", p.key_type)),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xbac2de))
                            .text_xs()
                            .font_family("monospace")
                            .child(format!("指纹: {}", p.fingerprint)),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x808080))
                            .text_xs()
                            .child("请校验 fingerprint 是否与服务器一致 (ssh-keygen -lf <key>)"),
                    )
                    .child(
                        div()
                            .mt(px(8.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(self.hostkey_btn("hostkey-trust-once", "信任一次", false, decision_id, cx))
                            .child(self.hostkey_btn("hostkey-trust-perm", "永久信任", true, decision_id, cx))
                            .child(self.hostkey_btn_red("hostkey-reject", "拒绝", false, decision_id, cx)),
                    ),
            );
        vec![gpui::IntoElement::into_any_element(overlay)]
    }

    fn hostkey_btn(
        &self,
        id: &'static str,
        label: &'static str,
        permanent: bool,
        decision_id: uuid::Uuid,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::AnyElement {
        // permanent=true 表示"永久信任"(accept=true,permanent=true)
        // permanent=false + accept 由 on_click 闭包决定 (信任一次 accept=true,perm=false; 拒绝 accept=false)
        let accept = label != "拒绝";
        gpui::IntoElement::into_any_element(
            div()
                .id((id, 0usize))
                .flex_1()
                .h(px(36.0))
                .bg(rgb(0x3b82f6))
                .rounded(px(4.0))
                .text_color(rgb(0xffffff))
                .text_sm()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0x2563eb)))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                        bridge.send_command(rshell_api::AppCommand::DecideHostKey {
                            decision_id,
                            accept,
                            permanent,
                        });
                    }
                    this.pending_host_key_prompt = None;
                }))
                .child(label),
        )
    }

    fn hostkey_btn_red(
        &self,
        id: &'static str,
        label: &'static str,
        permanent: bool,
        decision_id: uuid::Uuid,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::AnyElement {
        let accept = label != "拒绝";
        gpui::IntoElement::into_any_element(
            div()
                .id((id, 0usize))
                .flex_1()
                .h(px(36.0))
                .bg(rgb(0xdc2626))
                .rounded(px(4.0))
                .text_color(rgb(0xffffff))
                .text_sm()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0xb91c1c)))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                        bridge.send_command(rshell_api::AppCommand::DecideHostKey {
                            decision_id,
                            accept,
                            permanent,
                        });
                    }
                    this.pending_host_key_prompt = None;
                }))
                .child(label),
        )
    }
}
