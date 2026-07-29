//! 隧道管理面板视图
//!
//! 显示活动隧道列表，支持创建/关闭/暂停/恢复隧道操作。

use gpui::*;
use rshell_api::types::{ActiveTunnelInfo, ForwardDirection, TunnelState};
use rshell_api::AppEvent;

/// 隧道面板视图
pub struct TunnelPanelView {
    tunnels: Vec<ActiveTunnelInfo>,
    selected_tunnel: Option<usize>,
}

impl TunnelPanelView {
    /// 创建新的隧道面板视图
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            tunnels: Vec::new(),
            selected_tunnel: None,
        }
    }

    /// 更新隧道列表
    pub fn update_tunnels(&mut self, tunnels: Vec<ActiveTunnelInfo>) {
        self.tunnels = tunnels;
    }

    /// 处理事件
    pub fn handle_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::ActiveTunnelsChanged => {
                // 需要重新拉取隧道列表
            }
            AppEvent::TunnelUpdated { tunnel } => {
                if let Some(existing) = self.tunnels.iter_mut().find(|t| t.id == tunnel.id) {
                    *existing = tunnel.clone();
                } else {
                    self.tunnels.push(tunnel.clone());
                }
            }
            _ => {}
        }
    }
}

impl Render for TunnelPanelView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .child(
                div()
                    .h(px(40.0))
                    .bg(rgb(0x252526))
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .child(
                        div()
                            .child("隧道管理")
                            .text_color(rgb(0xcccccc))
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::BOLD),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(8.0))
                    .child(self.render_tunnel_list()),
            )
    }
}

impl TunnelPanelView {
    fn render_tunnel_list(&self) -> impl IntoElement {
        if self.tunnels.is_empty() {
            div()
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .child(
                    div()
                        .child("暂无活动隧道")
                        .text_color(rgb(0x808080))
                        .text_size(px(12.0)),
                )
        } else {
            div().children(self.tunnels.iter().enumerate().map(|(idx, tunnel)| {
                self.render_tunnel_item(idx, tunnel)
            }))
        }
    }

    fn render_tunnel_item(&self, idx: usize, tunnel: &ActiveTunnelInfo) -> impl IntoElement {
        let is_selected = self.selected_tunnel == Some(idx);
        let bg = if is_selected { rgb(0x094771) } else { rgb(0x2d2d2d) };
        let state_color = match &tunnel.state {
            TunnelState::Active => rgb(0x4ec9b0),
            TunnelState::Suspended => rgb(0xdcdcaa),
            TunnelState::Error(_) => rgb(0xf44747),
        };
        let state_text = match &tunnel.state {
            TunnelState::Active => "活动",
            TunnelState::Suspended => "暂停",
            TunnelState::Error(_) => "错误",
        };
        let direction_text = match tunnel.rule.direction {
            ForwardDirection::Local => "本地",
            ForwardDirection::Remote => "远程",
            ForwardDirection::Dynamic => "动态",
        };

        div()
            .bg(bg)
            .rounded(px(4.0))
            .mb(px(4.0))
            .p(px(8.0))
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(8.0))
                                    .h(px(8.0))
                                    .rounded(px(4.0))
                                    .bg(state_color),
                            )
                            .child(
                                div()
                                    .child(format!("{}:{}", tunnel.rule.bind_address, tunnel.rule.bind_port))
                                    .text_color(rgb(0xcccccc))
                                    .text_size(px(12.0)),
                            ),
                    )
                    .child(
                        div()
                            .child(state_text)
                            .text_color(state_color)
                            .text_size(px(10.0)),
                    ),
            )
            .child(
                div()
                    .mt(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .child(format!("方向: {}", direction_text))
                            .text_color(rgb(0x808080))
                            .text_size(px(10.0)),
                    )
                    .child(
                        div()
                            .child(format!("连接: {}", tunnel.connections_count))
                            .text_color(rgb(0x808080))
                            .text_size(px(10.0)),
                    )
                    .child(
                        div()
                            .child(format!("流量: {} B", tunnel.bytes_transferred))
                            .text_color(rgb(0x808080))
                            .text_size(px(10.0)),
                    ),
            )
    }
}
