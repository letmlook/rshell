//! 会话视图
//!
//! 会话树视图组件，显示会话列表和文件夹结构。

use gpui::*;
use rshell_api::types::{ConnectionState, SessionInfo};

/// 会话树视图组件
pub struct SessionView {
    sessions: Vec<SessionInfo>,
    selected_session: Option<usize>,
}

impl SessionView {
    /// 创建新的视图
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            sessions: Vec::new(),
            selected_session: None,
        }
    }

    /// 更新会话列表
    pub fn update_sessions(&mut self, sessions: Vec<SessionInfo>) {
        self.sessions = sessions;
    }

    /// 处理事件
    pub fn handle_event(&mut self, event: &rshell_api::AppEvent) {
        match event {
            rshell_api::AppEvent::SessionListChanged => {
                // 需要重新拉取会话列表
            }
            rshell_api::AppEvent::ConnectionStateChanged { session_id, state, info: _ } => {
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == *session_id) {
                    session.state = *state;
                }
            }
            _ => {}
        }
    }
}

impl Render for SessionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 在 render 内需要 &mut self 调用 render_session_item (里面有 on_click cx.listener)。
        // 用 take + replace 避免 borrow checker 冲突。
        let mut items: Vec<gpui::AnyElement> = Vec::with_capacity(self.sessions.len());
        let sessions = std::mem::take(&mut self.sessions);
        for (idx, session) in sessions.iter().enumerate() {
            items.push(gpui::IntoElement::into_any_element(self.render_session_item(idx, session, cx)));
        }
        // 写回
        let _ = std::mem::replace(&mut self.sessions, sessions);

        div()
            .size_full()
            .bg(rgb(0x181825))
            .child(
                div()
                    .h(px(36.0))
                    .bg(rgb(0x1e1e2e))
                    .flex()
                    .items_center()
                    .px(px(10.0))
                    .child(
                        div()
                            .child("会话管理")
                            .text_color(rgb(0xcccccc))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::BOLD),
                    ),
            )
            .child(div().flex_1().p(px(4.0)).children(items))
    }
}

impl SessionView {
    fn render_session_item(
        &mut self,
        idx: usize,
        session: &SessionInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_session == Some(idx);
        let bg = if is_selected {
            rgb(0x094771)
        } else {
            rgb(0x181825)
        };

        let (state_color, _state_text) = match session.state {
            ConnectionState::Connected => (rgb(0x4ec9b0), "已连接"),
            ConnectionState::Connecting | ConnectionState::Authenticating => (rgb(0xdcdcaa), "连接中"),
            ConnectionState::Disconnected => (rgb(0x808080), "未连接"),
            ConnectionState::Disconnecting => (rgb(0xdcdcaa), "断开中"),
        };

        let protocol_text = match session.config.protocol {
            rshell_api::types::Protocol::SSH => "SSH",
            rshell_api::types::Protocol::Telnet => "Telnet",
            rshell_api::types::Protocol::Serial => "Serial",
            rshell_api::types::Protocol::RDP => "RDP",
        };

        let session_id = session.id;
        let name = session.config.name.clone();
        let host = session.config.host.clone();
        let port = session.config.port;

        div()
            .id(("session-row", idx))
            .bg(bg)
            .rounded(px(3.0))
            .mb(px(2.0))
            .px(px(8.0))
            .py(px(4.0))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0x252535)))
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.selected_session = Some(idx);
                cx.notify();
                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                    bridge.send_command(rshell_api::AppCommand::ConnectSession {
                        session_id,
                    });
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded(px(3.0))
                            .bg(state_color),
                    )
                    .child(
                        div()
                            .child(name)
                            .text_color(rgb(0xcccccc))
                            .text_size(px(11.0)),
                    ),
            )
            .child(
                div()
                    .ml(px(12.0))
                    .child(format!("{} {}:{}", protocol_text, host, port))
                    .text_color(rgb(0x606060))
                    .text_size(px(9.0)),
            )
    }
}
