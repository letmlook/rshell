//! 撰写窗格视图
//!
//! 多行文本编辑器，支持将文本批量发送到目标会话。
//! 可选择发送到当前会话、所有会话或选中会话。

use gpui::*;
use rshell_api::types::ComposeTarget;
use uuid::Uuid;

/// 撰写窗格视图
pub struct ComposePaneView {
    /// 编辑内容
    content: String,
    /// 发送目标
    target: ComposeTarget,
    /// 发送历史
    history: Vec<String>,
    /// 当前活动会话 ID
    active_session: Option<Uuid>,
    /// 光标位置（行，列）
    cursor_pos: (usize, usize),
}

impl ComposePaneView {
    /// 创建新的撰写窗格
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            content: String::new(),
            target: ComposeTarget::CurrentSession,
            history: Vec::new(),
            active_session: None,
            cursor_pos: (0, 0),
        }
    }

    /// 设置活动会话
    pub fn set_active_session(&mut self, session_id: Option<Uuid>) {
        self.active_session = session_id;
    }

    /// 设置内容
    pub fn set_content(&mut self, content: String) {
        self.content = content;
    }

    /// 获取内容
    pub fn content(&self) -> &str {
        &self.content
    }

    /// 获取发送目标
    pub fn target(&self) -> &ComposeTarget {
        &self.target
    }

    /// 切换发送目标
    pub fn cycle_target(&mut self) {
        self.target = match &self.target {
            ComposeTarget::CurrentSession => ComposeTarget::AllSessions,
            ComposeTarget::AllSessions => ComposeTarget::SelectedSessions(vec![]),
            ComposeTarget::SelectedSessions(_) => ComposeTarget::CurrentSession,
        };
    }

    /// 获取目标描述
    fn target_description(&self) -> &str {
        match &self.target {
            ComposeTarget::CurrentSession => "当前会话",
            ComposeTarget::AllSessions => "所有会话",
            ComposeTarget::SelectedSessions(_) => "选中会话",
        }
    }

    /// 获取内容行数
    fn line_count(&self) -> usize {
        if self.content.is_empty() {
            1
        } else {
            self.content.lines().count().max(1)
        }
    }
}

impl Render for ComposePaneView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let target_desc = self.target_description();
        let line_count = self.line_count();

        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .flex()
            .flex_col()
            // 工具栏
            .child(
                div()
                    .h(px(36.0))
                    .bg(rgb(0x181825))
                    .flex()
                    .items_center()
                    .px(px(8.0))
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_color(rgb(0xcdd6f4))
                            .text_sm()
                            .child("撰写窗格"),
                    )
                    .child(
                        div()
                            .flex_1(),
                    )
                    // 目标选择
                    .child(
                        div()
                            .bg(rgb(0x313244))
                            .rounded(px(4.0))
                            .px(px(8.0))
                            .py(px(4.0))
                            .child(
                                div()
                                    .text_color(rgb(0x89b4fa))
                                    .text_xs()
                                    .child(format!("目标: {}", target_desc)),
                            ),
                    )
                    // 发送按钮
                    .child(
                        div()
                            .bg(rgb(0x3b82f6))
                            .rounded(px(4.0))
                            .px(px(12.0))
                            .py(px(4.0))
                            .child(
                                div()
                                    .text_color(rgb(0xffffff))
                                    .text_xs()
                                    .child("发送"),
                            ),
                    ),
            )
            // 编辑区域
            .child(
                div()
                    .flex_1()
                    .bg(rgb(0x1e1e2e))
                    .flex()
                    .flex_row()
                    // 行号栏
                    .child(
                        div()
                            .w(px(40.0))
                            .bg(rgb(0x181825))
                            .flex()
                            .flex_col()
                            .children((1..=line_count).map(|i| {
                                div()
                                    .h(px(20.0))
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .pr(px(8.0))
                                    .child(
                                        div()
                                            .text_color(rgb(0x6c7086))
                                            .text_xs()
                                            .child(i.to_string()),
                                    )
                            })),
                    )
                    // 文本编辑区
                    .child(
                        div()
                            .flex_1()
                            .bg(rgb(0x1e1e2e))
                            .p(px(4.0))
                            .child(
                                if self.content.is_empty() {
                                    div()
                                        .text_color(rgb(0x6c7086))
                                        .text_sm()
                                        .child("在此输入要发送的文本...")
                                        .into_any()
                                } else {
                                    div()
                                        .text_color(rgb(0xcdd6f4))
                                        .font_family("Consolas")
                                        .text_sm()
                                        .child(self.content.clone())
                                        .into_any()
                                },
                            ),
                    ),
            )
            // 状态栏
            .child(
                div()
                    .h(px(24.0))
                    .bg(rgb(0x181825))
                    .flex()
                    .items_center()
                    .px(px(8.0))
                    .gap(px(12.0))
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .text_xs()
                            .child(format!("行数: {}", line_count)),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .text_xs()
                            .child(format!("字符: {}", self.content.len())),
                    )
                    .child(
                        div()
                            .flex_1(),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .text_xs()
                            .child("Ctrl+Enter 发送"),
                    ),
            )
    }
}
