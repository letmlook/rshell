//! SSH 密钥管理视图
//!
//! 显示密钥列表，支持生成/导入/导出/删除密钥操作。

use gpui::*;
use rshell_api::types::{SshKeyInfo, SshKeyType};
use rshell_api::AppEvent;
use std::cell::Cell;
use std::rc::Rc;

/// 密钥管理视图
pub struct KeyManagementView {
    keys: Vec<SshKeyInfo>,
    selected_key: Option<usize>,
    search_filter: String,
}

impl KeyManagementView {
    /// 创建新的密钥管理视图
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            keys: Vec::new(),
            selected_key: None,
            search_filter: String::new(),
        }
    }

    /// 更新密钥列表
    pub fn update_keys(&mut self, keys: Vec<SshKeyInfo>) {
        self.keys = keys;
    }

    /// 处理事件
    pub fn handle_event(&mut self, event: &AppEvent) {
        match event {
            AppEvent::SshKeyListChanged => {}
            AppEvent::SshKeyGenerated { key } => {
                self.keys.push(key.clone());
            }
            _ => {}
        }
    }

    /// 获取过滤后的密钥列表
    fn filtered_keys(&self) -> Vec<&SshKeyInfo> {
        if self.search_filter.is_empty() {
            self.keys.iter().collect()
        } else {
            let filter = self.search_filter.to_lowercase();
            self.keys
                .iter()
                .filter(|k| {
                    k.name.to_lowercase().contains(&filter)
                        || k.fingerprint.to_lowercase().contains(&filter)
                        || k.comment.to_lowercase().contains(&filter)
                })
                .collect()
        }
    }
}

impl Render for KeyManagementView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_key_id: Option<uuid::Uuid> = self
            .selected_key
            .and_then(|i| self.keys.get(i))
            .map(|k| k.id);
        // 用 Rc<Cell> 共享给闭包 (avoid &mut self borrow 逃逸)
        let selected_cell: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(self.selected_key));

        div()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .child(render_header(cx, selected_key_id))
            .child(
                div()
                    .flex_1()
                    .p(px(8.0))
                    .child(render_key_list(&self.keys, cx, selected_cell.clone())),
            )
    }
}

fn render_header(
    cx: &mut Context<KeyManagementView>,
    selected_key_id: Option<uuid::Uuid>,
) -> AnyElement {
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
                    .child("SSH 密钥管理")
                    .text_color(rgb(0xcccccc))
                    .text_size(px(14.0))
                    .font_weight(gpui::FontWeight::BOLD),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(header_btn(
                        "key-generate",
                        "生成",
                        cx,
                        |cx| {
                            if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                bridge.send_command(rshell_api::AppCommand::GenerateSshKey {
                                    name: "default".to_string(),
                                    key_type: rshell_api::types::SshKeyType::ED25519,
                                    passphrase: None,
                                });
                            }
                        },
                    ))
                    .child(header_btn(
                        "key-import",
                        "导入",
                        cx,
                        |cx| {
                            if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                bridge.send_command(
                                    rshell_api::AppCommand::ImportPrivateKey {
                                        path: std::path::PathBuf::from(""),
                                        passphrase: None,
                                    },
                                );
                            }
                        },
                    ))
                    .child(header_btn(
                        "key-export",
                        "导出",
                        cx,
                        move |cx| {
                            if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                if let Some(id) = selected_key_id {
                                    bridge.send_command(
                                        rshell_api::AppCommand::ExportPublicKey { key_id: id },
                                    );
                                }
                            }
                        },
                    )),
            ),
    )
}

fn header_btn(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<KeyManagementView>,
    on_click: impl Fn(&mut Context<KeyManagementView>) + 'static,
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

fn render_key_list(
    keys: &[SshKeyInfo],
    cx: &mut Context<KeyManagementView>,
    selected: Rc<Cell<Option<usize>>>,
) -> AnyElement {
    if keys.is_empty() {
        return IntoElement::into_any_element(
            div()
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .child(
                    div()
                        .child("暂无密钥")
                        .text_color(rgb(0x808080))
                        .text_size(px(12.0)),
                ),
        );
    }
    let keys_vec: Vec<SshKeyInfo> = keys.to_vec();
    let mut items: Vec<AnyElement> = Vec::with_capacity(keys_vec.len());
    for (idx, key) in keys_vec.iter().enumerate() {
        items.push(render_key_item(idx, key, selected.clone(), cx));
    }
    IntoElement::into_any_element(div().children(items))
}

fn render_key_item(
    idx: usize,
    key: &SshKeyInfo,
    selected: Rc<Cell<Option<usize>>>,
    cx: &mut Context<KeyManagementView>,
) -> AnyElement {
    let is_selected = selected.get() == Some(idx);
    let bg = if is_selected {
        rgb(0x094771)
    } else {
        rgb(0x2d2d2d)
    };

    let key_type_icon = match key.key_type {
        SshKeyType::ED25519 => "[K]",
        SshKeyType::RSA2048 | SshKeyType::RSA4096 => "[R]",
        SshKeyType::ECDSA256 | SshKeyType::ECDSA384 | SshKeyType::ECDSA521 => "[E]",
    };

    let key_type_text = match key.key_type {
        SshKeyType::ED25519 => "ED25519",
        SshKeyType::RSA2048 => "RSA-2048",
        SshKeyType::RSA4096 => "RSA-4096",
        SshKeyType::ECDSA256 => "ECDSA-256",
        SshKeyType::ECDSA384 => "ECDSA-384",
        SshKeyType::ECDSA521 => "ECDSA-521",
    };

    let key_id = key.id;
    let row_id: &'static str = Box::leak(format!("key-row-{}", key.id).into_boxed_str());
    let del_id: &'static str = Box::leak(format!("key-del-{}", key.id).into_boxed_str());

    let selected_for_row = selected.clone();

    IntoElement::into_any_element(
        div()
            .id((row_id, 0usize))
            .bg(bg)
            .rounded(px(4.0))
            .mb(px(4.0))
            .p(px(8.0))
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
                            .child(div().child(key_type_icon).text_size(px(14.0)))
                            .child(
                                div()
                                    .child(key.name.clone())
                                    .text_color(rgb(0xcccccc))
                                    .text_size(px(12.0))
                                    .font_weight(gpui::FontWeight::MEDIUM),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .bg(rgb(0x3c3c3c))
                                    .rounded(px(2.0))
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .child(
                                        div()
                                            .child(key_type_text)
                                            .text_color(rgb(0x4ec9b0))
                                            .text_size(px(10.0)),
                                    ),
                            )
                            .child(
                                div()
                                    .id((del_id, 0usize))
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded(px(2.0))
                                    .bg(rgb(0xf44747))
                                    .text_color(rgb(0xffffff))
                                    .text_xs()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0xcc0000)))
                                    .on_click(cx.listener(move |_, _, _window, cx| {
                                        if let Some(bridge) =
                                            cx.try_global::<crate::bridge::AppBridge>()
                                        {
                                            bridge.send_command(
                                                rshell_api::AppCommand::DeleteSshKey {
                                                    key_id,
                                                },
                                            );
                                        }
                                    }))
                                    .child("del"),
                            ),
                    ),
            )
            .child(
                div()
                    .mt(px(4.0))
                    .child(
                        div()
                            .child(key.fingerprint.clone())
                            .text_color(rgb(0x808080))
                            .text_size(px(10.0))
                            .font_family("monospace"),
                    ),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .child(format!(
                                "创建: {}",
                                &key.created_at[..10.min(key.created_at.len())]
                            ))
                            .text_color(rgb(0x606060))
                            .text_size(px(9.0)),
                    )
                    .child(if key.has_passphrase {
                        div()
                            .child("[locked]")
                            .text_color(rgb(0xdcdcaa))
                            .text_size(px(9.0))
                    } else {
                        div()
                            .child("[unlocked]")
                            .text_color(rgb(0x808080))
                            .text_size(px(9.0))
                    }),
            ),
    )
}
