//! 插件管理视图
//!
//! 显示已安装插件列表，支持加载/卸载/启用/禁用插件。

use gpui::*;
use rshell_api::types::{PluginInfo, PluginState, PluginType};
use rshell_api::AppEvent;
use std::cell::Cell;
use std::rc::Rc;

/// 插件管理视图
pub struct PluginManagerView {
    plugins: Vec<PluginInfo>,
    selected_plugin: Option<usize>,
}

impl PluginManagerView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            plugins: Vec::new(),
            selected_plugin: None,
        }
    }

    pub fn update_plugins(&mut self, plugins: Vec<PluginInfo>) {
        self.plugins = plugins;
    }

    pub fn handle_event(&mut self, event: &AppEvent) {
        match event {
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_cell: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(self.selected_plugin));
        let selected_id: Option<String> = self
            .selected_plugin
            .and_then(|i| self.plugins.get(i))
            .map(|p| p.id.clone());

        div()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .child(render_header(cx, selected_id, selected_cell.clone()))
            .child(
                div()
                    .flex_1()
                    .p(px(8.0))
                    .child(render_plugin_list(&self.plugins, cx, selected_cell)),
            )
    }
}

fn render_header(
    cx: &mut Context<PluginManagerView>,
    selected_id: Option<String>,
    selected_cell: Rc<Cell<Option<usize>>>,
) -> AnyElement {
    let sel_for_load = selected_id.clone();
    let sel_for_unload = selected_id.clone();
    let sel_for_enable = selected_id.clone();
    let sel_for_disable = selected_id.clone();
    let sel_cell_for_scan = selected_cell.clone();

    IntoElement::into_any_element(
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
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(action_btn(
                        "plugin-scan",
                        "扫描",
                        cx,
                        move |cx| {
                            if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                bridge.send_command(rshell_api::AppCommand::ScanPlugins);
                            }
                            sel_cell_for_scan.set(None);
                        },
                    ))
                    .child(action_btn(
                        "plugin-load",
                        "加载",
                        cx,
                        move |cx| {
                            if let Some(id) = sel_for_load.clone() {
                                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                    bridge
                                        .send_command(rshell_api::AppCommand::LoadPlugin { plugin_id: id });
                                }
                            }
                        },
                    ))
                    .child(action_btn(
                        "plugin-enable",
                        "启用",
                        cx,
                        move |cx| {
                            if let Some(id) = sel_for_enable.clone() {
                                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                    bridge.send_command(
                                        rshell_api::AppCommand::EnablePlugin { plugin_id: id },
                                    );
                                }
                            }
                        },
                    ))
                    .child(action_btn(
                        "plugin-disable",
                        "禁用",
                        cx,
                        move |cx| {
                            if let Some(id) = sel_for_disable.clone() {
                                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                    bridge.send_command(
                                        rshell_api::AppCommand::DisablePlugin { plugin_id: id },
                                    );
                                }
                            }
                        },
                    ))
                    .child(action_btn(
                        "plugin-unload",
                        "卸载",
                        cx,
                        move |cx| {
                            if let Some(id) = sel_for_unload.clone() {
                                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                    bridge.send_command(
                                        rshell_api::AppCommand::UnloadPlugin { plugin_id: id },
                                    );
                                }
                            }
                        },
                    )),
            ),
    )
}

fn action_btn(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<PluginManagerView>,
    on_click: impl Fn(&mut Context<PluginManagerView>) + 'static,
) -> AnyElement {
    IntoElement::into_any_element(
        div()
            .id((id, 0usize))
            .px(px(8.0))
            .py(px(2.0))
            .rounded(px(3.0))
            .bg(rgb(0x3b82f6))
            .text_color(rgb(0xffffff))
            .text_xs()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0x2563eb)))
            .on_click(cx.listener(move |_, _, _window, cx| {
                on_click(cx);
            }))
            .child(label),
    )
}

fn render_plugin_list(
    plugins: &[PluginInfo],
    cx: &mut Context<PluginManagerView>,
    selected: Rc<Cell<Option<usize>>>,
) -> AnyElement {
    if plugins.is_empty() {
        return IntoElement::into_any_element(
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
                ),
        );
    }
    let p: Vec<PluginInfo> = plugins.to_vec();
    let mut items: Vec<AnyElement> = Vec::with_capacity(p.len());
    for (idx, plugin) in p.iter().enumerate() {
        items.push(render_plugin_item(idx, plugin, selected.clone(), cx));
    }
    IntoElement::into_any_element(div().children(items))
}

fn render_plugin_item(
    idx: usize,
    plugin: &PluginInfo,
    selected: Rc<Cell<Option<usize>>>,
    cx: &mut Context<PluginManagerView>,
) -> AnyElement {
    let is_selected = selected.get() == Some(idx);
    let bg = if is_selected {
        rgb(0x094771)
    } else {
        rgb(0x2d2d2d)
    };

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

    let row_id: &'static str = Box::leak(format!("plugin-row-{}", plugin.id).into_boxed_str());
    let selected_for_row = selected.clone();

    IntoElement::into_any_element(
        div()
            .id((row_id, 0usize))
            .bg(bg)
            .rounded(px(4.0))
            .mb(px(4.0))
            .p(px(10.0))
            .cursor_pointer()
            .on_click(cx.listener(move |_, _, _window, _cx| {
                selected_for_row.set(Some(idx));
            }))
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
            ),
    )
}
