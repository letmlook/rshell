//! 传输视图
//!
//! 文件传输管理视图，显示传输任务列表和进度。

use gpui::*;
use crate::view_models::transfer_vm::{TransferViewModel, TransferDirection};

/// 文件传输视图组件
pub struct TransferView {
    /// 传输 ViewModel
    vm: TransferViewModel,
}

impl TransferView {
    /// 创建新的视图
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            vm: TransferViewModel::new(),
        }
    }

    /// 获取 ViewModel 引用
    pub fn view_model(&self) -> &TransferViewModel {
        &self.vm
    }

    /// 处理事件
    pub fn handle_event(&mut self, event: &rshell_api::AppEvent) {
        self.vm.handle_event(event);
    }
}

impl Render for TransferView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .child(
                div()
                    .h(px(36.0))
                    .bg(rgb(0x252526))
                    .flex()
                    .items_center()
                    .px(px(10.0))
                    .justify_between()
                    .child(
                        div()
                            .child("文件传输")
                            .text_color(rgb(0xcccccc))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::BOLD),
                    )
                    .child(
                        div()
                            .child(format!("{} 个任务", self.vm.tasks.len()))
                            .text_color(rgb(0x808080))
                            .text_size(px(10.0)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(4.0))
                    .children(self.vm.tasks.iter().map(|task| {
                        let progress = self.vm.progress_percent(task);
                        let dir_icon = match task.direction {
                            TransferDirection::Upload => "↑",
                            TransferDirection::Download => "↓",
                        };
                        let speed = TransferViewModel::format_speed(task.speed_bps);

                        div()
                            .bg(rgb(0x252526))
                            .rounded(px(4.0))
                            .mb(px(4.0))
                            .p(px(8.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(
                                        div()
                                            .child(dir_icon)
                                            .text_color(rgb(0x4ec9b0))
                                            .text_size(px(12.0)),
                                    )
                                    .child(
                                        div()
                                            .child(task.filename.clone())
                                            .text_color(rgb(0xcccccc))
                                            .text_size(px(11.0)),
                                    )
                                    .child(
                                        div()
                                            .ml(gpui::px(8.0))
                                            .child(task.state.clone())
                                            .text_color(rgb(0x808080))
                                            .text_size(px(9.0)),
                                    ),
                            )
                            .child(
                                div()
                                    .mt(px(4.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .child(format!("{:.1}%", progress))
                                            .text_color(rgb(0xdcdcaa))
                                            .text_size(px(9.0)),
                                    )
                                    .child(
                                        div()
                                            .child(speed)
                                            .text_color(rgb(0x808080))
                                            .text_size(px(9.0)),
                                    ),
                            )
                    })),
            )
    }
}
