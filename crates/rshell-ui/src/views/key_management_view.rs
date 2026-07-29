//! SSH 密钥管理视图
//!
//! 显示密钥列表，支持生成/导入/导出/删除密钥操作。

use gpui::*;
use rshell_api::types::{SshKeyInfo, SshKeyType};
use rshell_api::AppEvent;

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
            AppEvent::SshKeyListChanged => {
                // 需要重新拉取密钥列表
            }
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
                            .child("SSH 密钥管理")
                            .text_color(rgb(0xcccccc))
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::BOLD),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(8.0))
                    .child(self.render_key_list()),
            )
    }
}

impl KeyManagementView {
    fn render_key_list(&self) -> impl IntoElement {
        let filtered = self.filtered_keys();

        if filtered.is_empty() {
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
                )
        } else {
            div().children(filtered.iter().enumerate().map(|(idx, key)| {
                self.render_key_item(idx, key)
            }))
        }
    }

    fn render_key_item(&self, idx: usize, key: &SshKeyInfo) -> impl IntoElement {
        let is_selected = self.selected_key == Some(idx);
        let bg = if is_selected { rgb(0x094771) } else { rgb(0x2d2d2d) };

        let key_type_icon = match key.key_type {
            SshKeyType::ED25519 => "🔑",
            SshKeyType::RSA2048 | SshKeyType::RSA4096 => "🔐",
            SshKeyType::ECDSA256 | SshKeyType::ECDSA384 | SshKeyType::ECDSA521 => "🗝",
        };

        let key_type_text = match key.key_type {
            SshKeyType::ED25519 => "ED25519",
            SshKeyType::RSA2048 => "RSA-2048",
            SshKeyType::RSA4096 => "RSA-4096",
            SshKeyType::ECDSA256 => "ECDSA-256",
            SshKeyType::ECDSA384 => "ECDSA-384",
            SshKeyType::ECDSA521 => "ECDSA-521",
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
                                    .child(key_type_icon)
                                    .text_size(px(14.0)),
                            )
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
                            .child(format!("创建: {}", &key.created_at[..10.min(key.created_at.len())]))
                            .text_color(rgb(0x606060))
                            .text_size(px(9.0)),
                    )
                    .child(
                        if key.has_passphrase {
                            div()
                                .child("🔒 有密码")
                                .text_color(rgb(0xdcdcaa))
                                .text_size(px(9.0))
                        } else {
                            div()
                                .child("🔓 无密码")
                                .text_color(rgb(0x808080))
                                .text_size(px(9.0))
                        },
                    ),
            )
    }
}
