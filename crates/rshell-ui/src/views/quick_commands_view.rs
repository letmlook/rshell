//! 快速命令面板视图
//!
//! 展示所有快速命令，支持点击执行。
//! 按分组显示，支持搜索过滤。

use gpui::*;
use rshell_api::types::QuickCommand;
use uuid::Uuid;

/// 快速命令面板视图
pub struct QuickCommandsView {
    /// 快速命令列表
    commands: Vec<QuickCommand>,
    /// 搜索过滤文本
    filter: String,
    /// 当前活动会话 ID
    active_session: Option<Uuid>,
}

impl QuickCommandsView {
    /// 创建新的快速命令面板
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            commands: Vec::new(),
            filter: String::new(),
            active_session: None,
        }
    }

    /// 更新快速命令列表
    pub fn update_commands(&mut self, commands: Vec<QuickCommand>) {
        self.commands = commands;
    }

    /// 设置活动会话
    pub fn set_active_session(&mut self, session_id: Option<Uuid>) {
        self.active_session = session_id;
    }

    /// 获取过滤后的命令列表
    fn filtered_commands(&self) -> Vec<&QuickCommand> {
        if self.filter.is_empty() {
            return self.commands.iter().collect();
        }

        let filter_lower = self.filter.to_lowercase();
        self.commands
            .iter()
            .filter(|cmd| {
                cmd.name.to_lowercase().contains(&filter_lower)
                    || cmd.command.to_lowercase().contains(&filter_lower)
                    || cmd.description.to_lowercase().contains(&filter_lower)
            })
            .collect()
    }

    /// 按分组整理命令
    fn grouped_commands(&self) -> Vec<(Option<String>, Vec<&QuickCommand>)> {
        let filtered = self.filtered_commands();
        let mut groups: std::collections::HashMap<Option<String>, Vec<&QuickCommand>> =
            std::collections::HashMap::new();

        for cmd in filtered {
            groups
                .entry(cmd.group.clone())
                .or_default()
                .push(cmd);
        }

        let mut result: Vec<(Option<String>, Vec<&QuickCommand>)> = groups.into_iter().collect();
        result.sort_by(|a, b| {
            match (&a.0, &b.0) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (Some(a_name), Some(b_name)) => a_name.cmp(b_name),
            }
        });

        result
    }
}

impl Render for QuickCommandsView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let groups = self.grouped_commands();
        let filter_text = self.filter.clone();

        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .flex()
            .flex_col()
            // 标题栏
            .child(
                div()
                    .h(px(40.0))
                    .bg(rgb(0x181825))
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .child(
                        div()
                            .text_color(rgb(0xcdd6f4))
                            .text_sm()
                            .child("快速命令"),
                    ),
            )
            // 搜索栏
            .child(
                div()
                    .h(px(36.0))
                    .bg(rgb(0x1e1e2e))
                    .px(px(8.0))
                    .py(px(4.0))
                    .child(
                        div()
                            .bg(rgb(0x313244))
                            .rounded(px(4.0))
                            .px(px(8.0))
                            .py(px(4.0))
                            .child(
                                div()
                                    .text_color(rgb(0x6c7086))
                                    .text_sm()
                                    .child(if filter_text.is_empty() {
                                        "搜索命令...".to_string()
                                    } else {
                                        filter_text
                                    }),
                            ),
                    ),
            )
            // 命令列表
            .child(
                div()
                    .flex_1()
                    .children(groups.into_iter().flat_map(|(group_name, cmds)| {
                        let mut elements = Vec::new();

                        // 分组标题
                        if let Some(name) = group_name {
                            elements.push(
                                div()
                                    .h(px(24.0))
                                    .bg(rgb(0x181825))
                                    .px(px(12.0))
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_color(rgb(0x6c7086))
                                            .text_xs()
                                            .child(name),
                                    )
                                    .into_any(),
                            );
                        }

                        // 命令按钮
                        for cmd in cmds {
                            let scope_icon = match cmd.scope {
                                rshell_api::types::QuickCommandScope::CurrentSession => "▶",
                                rshell_api::types::QuickCommandScope::AllSessions => "▶▶",
                                rshell_api::types::QuickCommandScope::SelectedSessions(_) => "▶#",
                            };

                            elements.push(
                                div()
                                    .h(px(32.0))
                                    .px(px(12.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .hover(|this| this.bg(rgb(0x313244)))
                                    .child(
                                        div()
                                            .text_color(rgb(0x89b4fa))
                                            .text_xs()
                                            .child(scope_icon),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .child(
                                                div()
                                                    .text_color(rgb(0xcdd6f4))
                                                    .text_sm()
                                                    .child(cmd.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_color(rgb(0x6c7086))
                                                    .text_xs()
                                                    .child(cmd.command.chars().take(40).collect::<String>()),
                                            ),
                                    )
                                    .into_any(),
                            );
                        }

                        elements
                    })),
            )
    }
}
