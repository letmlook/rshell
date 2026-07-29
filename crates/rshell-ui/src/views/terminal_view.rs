//! 终端视图
//!
//! 渲染终端单元格，支持字符显示、颜色、样式、光标、选区。

use gpui::{div, prelude::*, rgb, Rgba, Window};
use rshell_api::types::{CellFlags, TerminalBufferSnapshot};

/// 选区（行/列坐标）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor_row: usize,
    pub anchor_col: usize,
    pub head_row: usize,
    pub head_col: usize,
}

/// 终端视图组件
pub struct TerminalView {
    /// 终端缓冲区快照
    buffer: Option<TerminalBufferSnapshot>,
    /// 单元格宽度（像素）
    cell_width: f32,
    /// 单元格高度（像素）
    cell_height: f32,
    /// 当前选区（None 表示无选区）
    selection: Option<Selection>,
}

impl TerminalView {
    /// 创建新的视图
    pub fn new() -> Self {
        Self {
            buffer: None,
            cell_width: 8.0,
            cell_height: 16.0,
            selection: None,
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

    /// 设置选区
    pub fn set_selection(&mut self, sel: Option<Selection>) {
        self.selection = sel;
    }

    /// 检查某 cell 是否在选区内
    fn is_in_selection(&self, row: usize, col: usize, sel: Selection) -> bool {
        let pos = (row, col);
        let anchor = (sel.anchor_row, sel.anchor_col);
        let head = (sel.head_row, sel.head_col);
        let (top, bottom) = if anchor <= head { (anchor, head) } else { (head, anchor) };
        pos >= top && pos <= bottom
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

                                // 选区高亮：覆盖背景色
                                let effective_bg = if let Some(sel) = self.selection {
                                    if self.is_in_selection(row, col, sel) {
                                        Rgba {
                                            r: 0.4,
                                            g: 0.5,
                                            b: 0.7,
                                            a: 1.0,
                                        }
                                    } else {
                                        bg
                                    }
                                } else {
                                    bg
                                };

                                // CellFlags 样式（行级粗粒度：基于当前行任一 cell 的 flags）
                                let mut cell_div = div()
                                    .w(gpui::px(self.cell_width))
                                    .h(gpui::px(self.cell_height))
                                    .bg(effective_bg)
                                    .text_color(fg)
                                    .flex()
                                    .items_center()
                                    .justify_center();

                                // 单 cell 级别的 italic / underline（更细粒度）
                                if cell.flags.contains(CellFlags::ITALIC) {
                                    cell_div = cell_div.italic();
                                }
                                if cell.flags.contains(CellFlags::UNDERLINE) {
                                    cell_div = cell_div.underline();
                                }
                                if cell.flags.contains(CellFlags::STRIKETHROUGH) {
                                    // gpui 0.2 无 strike API，使用 border 模拟
                                    cell_div = cell_div.border_b_2();
                                }

                                cell_div = if ch == ' ' || ch == '\0' {
                                    cell_div.child(div())
                                } else {
                                    cell_div.child(ch.to_string())
                                };

                                row_div = row_div.child(cell_div);
                            }
                        }

                        // 行级 bold：如果该行有任一 cell 含 BOLD，整行加粗
                        let row_has_bold = (0..buffer.cols).any(|col| {
                            let idx = row * buffer.cols + col;
                            idx < buffer.cells.len()
                                && buffer.cells[idx]
                                    .flags
                                    .contains(CellFlags::BOLD)
                        });
                        if row_has_bold {
                            row_div = row_div.font_weight(gpui::FontWeight::BOLD);
                        }

                        container = container.child(row_div);
                    }

                    // 光标层：绝对定位覆盖在网格之上
                    if buffer.cursor_visible
                        && buffer.cursor_row < buffer.rows
                        && buffer.cursor_col < buffer.cols
                    {
                        let cursor_x = buffer.cursor_col as f32 * self.cell_width;
                        let cursor_y = buffer.cursor_row as f32 * self.cell_height;
                        container = container.child(
                            div()
                                .absolute()
                                .left(gpui::px(cursor_x))
                                .top(gpui::px(cursor_y))
                                .w(gpui::px(self.cell_width))
                                .h(gpui::px(self.cell_height))
                                .bg(rgb(0xcdd6f4)),
                        );
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
