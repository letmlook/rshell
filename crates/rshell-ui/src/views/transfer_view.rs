//! 传输视图
//!
//! 文件传输管理视图，显示传输任务列表和进度。

use gpui::*;
use crate::view_models::transfer_vm::{TransferDirection, TransferTaskView};

/// 文件传输视图组件
pub struct TransferView {
    /// 传输任务列表 (从 VM 拉取的快照)
    tasks: Vec<TransferTaskView>,
}

impl TransferView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self { tasks: Vec::new() }
    }

    /// 处理事件: 转给 view 自己维护的 tasks 列表 (从 vm handle_event 同步)
    pub fn handle_event(&mut self, event: &rshell_api::AppEvent) {
        match event {
            rshell_api::AppEvent::TransferTaskAdded { task_id, filename, direction } => {
                let dir = match direction {
                    rshell_api::types::TransferDirection::Upload => TransferDirection::Upload,
                    rshell_api::types::TransferDirection::Download => TransferDirection::Download,
                };
                if !self.tasks.iter().any(|t| t.task_id == *task_id) {
                    self.tasks.push(TransferTaskView {
                        task_id: *task_id,
                        filename: filename.clone(),
                        direction: dir,
                        bytes_transferred: 0,
                        total_bytes: 0,
                        speed_bps: 0.0,
                        state: "排队中".to_string(),
                    });
                }
            }
            rshell_api::AppEvent::TransferProgress { task_id, bytes, total, speed_bps } => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.task_id == *task_id) {
                    t.bytes_transferred = *bytes;
                    t.total_bytes = *total;
                    t.speed_bps = *speed_bps;
                }
            }
            rshell_api::AppEvent::TransferTaskCompleted { task_id } => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.task_id == *task_id) {
                    t.state = "已完成".to_string();
                }
            }
            rshell_api::AppEvent::TransferTaskFailed { task_id, error } => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.task_id == *task_id) {
                    t.state = format!("失败: {}", error);
                }
            }
            _ => {}
        }
    }
}

impl Render for TransferView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.tasks.len();
        let tasks: Vec<TransferTaskView> = self.tasks.clone();
        let mut items: Vec<AnyElement> = Vec::with_capacity(tasks.len());
        for task in &tasks {
            items.push(render_task(task, cx));
        }

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
                            .child(format!("{} 个任务", count))
                            .text_color(rgb(0x808080))
                            .text_size(px(10.0)),
                    ),
            )
            .child(div().flex_1().p(px(4.0)).children(items))
    }
}

fn render_task(task: &TransferTaskView, cx: &mut Context<TransferView>) -> AnyElement {
    let progress = if task.total_bytes > 0 {
        (task.bytes_transferred as f64 / task.total_bytes as f64) * 100.0
    } else {
        0.0
    };
    let dir_icon = match task.direction {
        TransferDirection::Upload => "↑",
        TransferDirection::Download => "↓",
    };
    let speed_bps = task.speed_bps;
    let speed_text = if speed_bps < 1024.0 {
        format!("{:.0} B/s", speed_bps)
    } else if speed_bps < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", speed_bps / 1024.0)
    } else {
        format!("{:.2} MB/s", speed_bps / 1024.0 / 1024.0)
    };

    let pause_id: &'static str = Box::leak(format!("transfer-pause-{}", task.task_id).into_boxed_str());
    let resume_id: &'static str = Box::leak(format!("transfer-resume-{}", task.task_id).into_boxed_str());
    let cancel_id: &'static str = Box::leak(format!("transfer-cancel-{}", task.task_id).into_boxed_str());

    IntoElement::into_any_element(
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
                    .child(div().child(dir_icon).text_color(rgb(0x4ec9b0)).text_size(px(12.0)))
                    .child(div().child(task.filename.clone()).text_color(rgb(0xcccccc)).text_size(px(11.0)))
                    .child(div().ml(px(8.0)).child(task.state.clone()).text_color(rgb(0x808080)).text_size(px(9.0))),
            )
            .child(
                div()
                    .mt(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().child(format!("{:.1}%", progress)).text_color(rgb(0xdcdcaa)).text_size(px(9.0)))
                    .child(div().child(speed_text).text_color(rgb(0x808080)).text_size(px(9.0))),
            )
            .child(
                div()
                    .mt(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(task_btn(pause_id, "暂停", task.task_id, "Pause", cx))
                    .child(task_btn(resume_id, "恢复", task.task_id, "Resume", cx))
                    .child(task_btn(cancel_id, "取消", task.task_id, "Cancel", cx)),
            ),
    )
}

fn task_btn(
    id: &'static str,
    label: &'static str,
    task_id: uuid::Uuid,
    action: &'static str,
    cx: &mut Context<TransferView>,
) -> AnyElement {
    let cmd = match action {
        "Pause" => rshell_api::AppCommand::PauseTransfer { task_id },
        "Resume" => rshell_api::AppCommand::ResumeTransfer { task_id },
        _ => rshell_api::AppCommand::CancelTransfer { task_id },
    };
    IntoElement::into_any_element(
        div()
            .id((id, 0usize))
            .px(px(6.0))
            .py(px(1.0))
            .rounded(px(2.0))
            .bg(rgb(0x3b82f6))
            .text_color(rgb(0xffffff))
            .text_xs()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0x2563eb)))
            .on_click(cx.listener(move |_, _, _window, cx| {
                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                    bridge.send_command(cmd.clone());
                }
            }))
            .child(label),
    )
}
