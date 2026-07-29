//! 终端视图
//!
//! 渲染终端单元格，支持字符显示、颜色、样式。

use gpui::{div, prelude::*, rgb, Window};
use rshell_api::types::TerminalBufferSnapshot;

/// 终端视图组件
pub struct TerminalView {
    /// 终端缓冲区快照
    buffer: Option<TerminalBufferSnapshot>,
    /// 单元格宽度（像素）
    cell_width: f32,
    /// 单元格高度（像素）
    cell_height: f32,
}

impl TerminalView {
    /// 创建新的视图
    pub fn new() -> Self {
        Self {
            buffer: None,
            cell_width: 8.0,
            cell_height: 16.0,
        }
    }

    /// 更新缓冲区
    pub fn update_buffer(&mut self, snapshot: TerminalBufferSnapshot) {
        self.buffer = Some(snapshot);
    }

    /// 设置单元格大小
    pub fn set_cell_size(&mut self, width: f32, height: f32) {
        self.cell_width = width;
        self.cell_height = height;
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let buffer = self.buffer.as_ref();

        div()
            .size_full()
            .bg(rgb(0x000000))
            .font_family("Consolas")
            .text_size(gpui::px(self.cell_height))
            .map(|_this| {
                if let Some(buffer) = buffer {
                    // 渲染终端内容
                    let mut container = div().size_full();

                    for row in 0..buffer.rows {
                        let mut row_div = div().flex().flex_row();

                        for col in 0..buffer.cols {
                            let idx = row * buffer.cols + col;
                            if idx < buffer.cells.len() {
                                let cell = &buffer.cells[idx];
                                let fg = rgb_color(cell.fg_color);
                                let bg = rgb_color(cell.bg_color);
                                let ch = cell.character;

                                row_div = row_div.child(
                                    div()
                                        .w(gpui::px(self.cell_width))
                                        .h(gpui::px(self.cell_height))
                                        .bg(bg)
                                        .text_color(fg)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(if ch == ' ' || ch == '\0' {
                                            div().into_any()
                                        } else {
                                            div().child(ch.to_string()).into_any()
                                        }),
                                );
                            }
                        }

                        container = container.child(row_div);
                    }

                    container
                } else {
                    // 无缓冲区时显示占位符
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(0x888888))
                        .child("No terminal session")
                }
            })
    }
}

/// 转换 RGBA 颜色
fn rgb_color(color: [u8; 4]) -> gpui::Rgba {
    gpui::Rgba {
        r: color[0] as f32 / 255.0,
        g: color[1] as f32 / 255.0,
        b: color[2] as f32 / 255.0,
        a: color[3] as f32 / 255.0,
    }
}
