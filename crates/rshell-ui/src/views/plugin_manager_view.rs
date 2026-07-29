//! 插件管理视图
//!
//! 显示已安装插件列表，支持加载/卸载/启用/禁用插件。

use gpui::*;
use rshell_api::types::{PluginInfo, PluginState, PluginType};
use rshell_api::AppEvent;

/// 插件管理视图
pub struct PluginManagerView {
    /// 插件列表
    plugins: Vec<PluginInfo>,
    /// 选中的插件索引
    selected_plugin: Option<usize>,
}

impl PluginManagerView {
    /// 创建新的插件管理视图
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            plugins: Vec::new(),
            selected_plugin: None,
        }
    }

    /// 更新插件列表
    pub fn update_plugins(&mut self, plugins: Vec<PluginInfo>) {
        self.plugins = plugins;
    }

    /// 处理事件
    pub fn handle_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::PluginListUpdated => {
                // 需要重新拉取插件列表
            }
            AppEvent::PluginStateChanged { plugin_id, state } => {
                if let Some(plugin) = self.plugins.iter_mut().find(|p| p.id == *plugin_id) {
                    plugin.state = *state;
                }
            }
            _ => {}
        }
    }
}

impl Render for PluginManagerView {
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
                    .justify_between()
                    .child(
                        div()
                            .child("插件管理")
                            .text_color(rgb(0xcccccc))
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::BOLD),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(8.0))
                    .child(self.render_plugin_list()),
            )
    }
}

impl PluginManagerView {
    fn render_plugin_list(&self) -> impl IntoElement {
        if self.plugins.is_empty() {
            div()
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .child("暂无已安装插件")
                                .text_color(rgb(0x808080))
                                .text_size(px(12.0)),
                        )
                        .child(
                            div()
                                .child("将插件放入 ~/.rshell/plugins/ 目录")
                                .text_color(rgb(0x606060))
                                .text_size(px(10.0)),
                        ),
                )
        } else {
            div().children(self.plugins.iter().enumerate().map(|(idx, plugin)| {
                self.render_plugin_item(idx, plugin)
            }))
        }
    }

    fn render_plugin_item(&self, idx: usize, plugin: &PluginInfo) -> impl IntoElement {
        let is_selected = self.selected_plugin == Some(idx);
        let bg = if is_selected { rgb(0x094771) } else { rgb(0x2d2d2d) };

        let (state_color, state_text) = match plugin.state {
            PluginState::Active => (rgb(0x4ec9b0), "已启用"),
            PluginState::Loaded => (rgb(0xdcdcaa), "已加载"),
            PluginState::Discovered => (rgb(0x808080), "已发现"),
            PluginState::Error => (rgb(0xf44747), "错误"),
            PluginState::Disabled => (rgb(0x606060), "已禁用"),
        };

        let type_text = match plugin.plugin_type {
            PluginType::Builtin => "内置",
            PluginType::Wasm => "WASM",
            PluginType::DynamicLib => "原生",
        };

        div()
            .bg(bg)
            .rounded(px(4.0))
            .mb(px(4.0))
            .p(px(10.0))
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
                                    .child(plugin.name.clone())
                                    .text_color(rgb(0xcccccc))
                                    .text_size(px(12.0))
                                    .font_weight(gpui::FontWeight::BOLD),
                            )
                            .child(
                                div()
                                    .child(format!("v{}", plugin.version))
                                    .text_color(rgb(0x808080))
                                    .text_size(px(10.0)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .child(type_text)
                                    .text_color(rgb(0x569cd6))
                                    .text_size(px(9.0))
                                    .bg(rgb(0x1e3a5f))
                                    .rounded(px(2.0))
                                    .px(px(4.0))
                                    .py(px(1.0)),
                            )
                            .child(
                                div()
                                    .child(state_text)
                                    .text_color(state_color)
                                    .text_size(px(10.0)),
                            ),
                    ),
            )
            .child(
                div()
                    .mt(px(4.0))
                    .child(plugin.description.clone())
                    .text_color(rgb(0x808080))
                    .text_size(px(10.0)),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .child(format!("作者: {}", plugin.author))
                    .text_color(rgb(0x606060))
                    .text_size(px(9.0)),
            )
    }
}
