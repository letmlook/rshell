//! 撰写窗格视图
//!
//! 多行文本编辑器，支持将文本批量发送到目标会话。
//! 使用 gpui_component::input 提供真实的多行编辑能力。
//! 可选择发送到当前会话、所有会话或选中会话。

use gpui::*;
use gpui_component::input::{Input, InputState};
use rshell_api::types::ComposeTarget;
use uuid::Uuid;

/// 撰写窗格视图
pub struct ComposePaneView {
    /// 编辑内容状态（gpui_component，懒创建 — 在首次 render() 时构造，
    /// 因为 InputState::new 需要 `&mut Window`，而 cx.new 闭包拿不到）
    input_state: Option<Entity<InputState>>,
    /// 发送目标
    target: ComposeTarget,
    /// 发送历史
    history: Vec<String>,
    /// 当前活动会话 ID
    active_session: Option<Uuid>,
}

impl ComposePaneView {
    /// 创建新的撰写窗格（InputState 在首次 render 时懒构造）
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            input_state: None,
            target: ComposeTarget::CurrentSession,
            history: Vec::new(),
            active_session: None,
        }
    }

    /// 设置活动会话
    pub fn set_active_session(&mut self, session_id: Option<Uuid>) {
        self.active_session = session_id;
    }

    /// 获取 InputState（懒构造）
    fn ensure_input_state(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if self.input_state.is_none() {
            let state = cx.new(|cx: &mut Context<InputState>| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .placeholder("在此输入要发送的文本...")
            });
            self.input_state = Some(state);
        }
        self.input_state.clone().unwrap()
    }

    /// 获取内容
    pub fn content(&self, cx: &App) -> String {
        match &self.input_state {
            Some(s) => s.read(cx).value().to_string(),
            None => String::new(),
        }
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
}

impl Render for ComposePaneView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input_state = self.ensure_input_state(window, cx);
        let target_desc = self.target_description();
        let char_count = input_state.read(cx).value().len();

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
                    .child(div().flex_1())
                    // 目标选择 — 点击 cycle_target
                    .child(
                        div()
                            .id(("compose-target-chip", 0usize))
                            .bg(rgb(0x313244))
                            .rounded(px(4.0))
                            .px(px(8.0))
                            .py(px(4.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x45475a)))
                            .on_click(cx.listener(|this, _, _window, _cx| {
                                this.cycle_target();
                            }))
                            .child(
                                div()
                                    .text_color(rgb(0x89b4fa))
                                    .text_xs()
                                    .child(format!("目标: {} (点切换)", target_desc)),
                            ),
                    )
                    // 发送按钮
                    .child(
                        div()
                            .id(("compose-send-btn", 0usize))
                            .bg(rgb(0x3b82f6))
                            .rounded(px(4.0))
                            .px(px(12.0))
                            .py(px(4.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x2563eb)))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                let content = this.content(cx);
                                if content.trim().is_empty() {
                                    return;
                                }
                                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                    bridge.send_command(
                                        rshell_api::AppCommand::SendComposeText {
                                            content,
                                            target: this.target.clone(),
                                        },
                                    );
                                }
                            }))
                            .child(
                                div()
                                    .text_color(rgb(0xffffff))
                                    .text_xs()
                                    .child("发送"),
                            ),
                    ),
            )
            // 编辑区域（gpui_component Input）
            .child(
                div()
                    .flex_1()
                    .bg(rgb(0x1e1e2e))
                    .p(px(4.0))
                    .child(Input::new(&input_state)),
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
                            .child(format!("字符: {}", char_count)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .text_xs()
                            .child("Ctrl+Enter 发送"),
                    ),
            )
    }
}