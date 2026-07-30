//! 终端视图
//!
//! 渲染终端单元格，支持字符显示、颜色、样式、光标、选区、键盘输入。

use gpui::{div, prelude::*, rgb, App, FocusHandle, Rgba, Window};
use rshell_api::types::{CellFlags, TerminalBufferSnapshot};
use rshell_api::AppCommand;
use uuid::Uuid;

use crate::bridge::AppBridge;

/// 全局终端输入状态（由 RshellApp 在 tab 切换时更新；
/// TerminalView 的 key listener 读取这里拿到当前 session_id）
#[derive(Debug, Clone, Copy)]
pub struct TerminalInputState {
    pub session_id: Uuid,
}

// gpui::Global 实现 — 让 listener 通过 cx.global::<TerminalInputState>() 读取
impl gpui::Global for TerminalInputState {}

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
    /// 焦点句柄（用于接收键盘事件）
    focus_handle: FocusHandle,
    /// 当前激活 session id（用于构造 SendInput 命令）
    session_id: uuid::Uuid,
}

impl TerminalView {
    /// 创建新的视图
    pub fn new(cx: &mut gpui::Context<Self>) -> Self {
        Self {
            buffer: None,
            cell_width: 8.0,
            cell_height: 16.0,
            selection: None,
            focus_handle: cx.focus_handle(),
            session_id: uuid::Uuid::nil(),
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

    /// 设置当前会话 id（用于 SendInput）
    pub fn set_session_id(&mut self, id: uuid::Uuid) {
        self.session_id = id;
    }

    /// 检查某 cell 是否在选区内
    fn is_in_selection(&self, row: usize, col: usize, sel: Selection) -> bool {
        let pos = (row, col);
        let anchor = (sel.anchor_row, sel.anchor_col);
        let head = (sel.head_row, sel.head_col);
        let (top, bottom) = if anchor <= head { (anchor, head) } else { (head, anchor) };
        pos >= top && pos <= bottom
    }

    /// 把 keystroke 转为发往后端的字节序列
    fn keystroke_to_bytes(key: &str, key_char: Option<&str>, has_control: bool, has_alt: bool) -> Vec<u8> {
        // 控制字符（C0）：Enter -> CR, Tab, Backspace, Escape
        match key {
            "enter" | "return" => return vec![b'\r'],
            "tab" => return vec![b'\t'],
            "backspace" => return vec![0x7f],
            "escape" | "esc" => return vec![0x1b],
            "up" => return b"\x1b[A".to_vec(),
            "down" => return b"\x1b[B".to_vec(),
            "right" => return b"\x1b[C".to_vec(),
            "left" => return b"\x1b[D".to_vec(),
            "home" => return b"\x1b[H".to_vec(),
            "end" => return b"\x1b[F".to_vec(),
            "delete" | "del" => return b"\x1b[3~".to_vec(),
            "pageup" => return b"\x1b[5~".to_vec(),
            "pagedown" => return b"\x1b[6~".to_vec(),
            _ => {}
        }

        // Ctrl+字母 → 控制字符（0x01-0x1a）
        if has_control {
            if let Some(c) = key_char.and_then(|s| s.chars().next()) {
                if c.is_ascii_alphabetic() {
                    let upper = c.to_ascii_uppercase();
                    if upper.is_ascii_uppercase() {
                        return vec![upper as u8 - b'A' + 1];
                    }
                }
            }
        }

        // Alt+键 → ESC 前缀
        if has_alt {
            if let Some(s) = key_char {
                let mut bytes = vec![0x1b];
                bytes.extend_from_slice(s.as_bytes());
                return bytes;
            }
        }

        // 普通可打印字符
        if let Some(s) = key_char {
            return s.as_bytes().to_vec();
        }

        // 回退到 key 字符串（多数情况下 key == key_char）
        if !key.is_empty() && key.chars().all(|c| !c.is_control()) {
            key.as_bytes().to_vec()
        } else {
            Vec::new()
        }
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
            .track_focus(&self.focus_handle)
            // 右键 -> 复制当前 session 的 buffer (后端 CopySelection 命令
            // 提取 alacritty 的当前 selection 字符串; 我们这里只是 dispatch,
            // 不必真在 view 里维护 selection start/end 坐标 — 后端 snapshot
            // 里有当前已选文本, 之前是空 selection
            .on_mouse_down(gpui::MouseButton::Right, move |_event: &gpui::MouseDownEvent, _window, cx: &mut App| {
                {
                    let bridge = cx.global::<AppBridge>().clone();
                    let session_id = cx
                        .try_global::<TerminalInputState>()
                        .map(|s| s.session_id)
                        .unwrap_or_else(uuid::Uuid::nil);
                    if session_id.is_nil() {
                        return;
                    }
                    // CopySelection 让后端去取 buffer selection; 用户要在
                    // 终端里选区 (左键拖动) 后右键才有效. 这个路径对全屏应用
                    // 是简化版, 实际 copy 走 "全选 + 复制" 流程.
                    bridge.send_command(AppCommand::CopySelection { session_id });
                }
            })
            .on_key_down(|event, _window, app: &mut App| {
                // 转换 keystroke → 字节序列
                let key = event.keystroke.key.as_str();
                let key_char = event.keystroke.key_char.as_deref();
                let has_control = event.keystroke.modifiers.control;
                let has_alt = event.keystroke.modifiers.alt;

                let bytes = TerminalView::keystroke_to_bytes(key, key_char, has_control, has_alt);
                if bytes.is_empty() {
                    return;
                }

                // 通过 global 拿到 AppBridge + 当前 session_id
                // (session_id 由 RshellApp 在 tab 切换时更新到 global)
                let bridge = app.global::<AppBridge>().clone();
                let session_id = app
                    .try_global::<TerminalInputState>()
                    .map(|s| s.session_id)
                    .unwrap_or_else(uuid::Uuid::nil);

                if session_id.is_nil() {
                    // 尚未选择激活的 tab，丢弃输入
                    return;
                }

                bridge.send_command(AppCommand::SendInput { session_id, data: bytes });
            })
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
