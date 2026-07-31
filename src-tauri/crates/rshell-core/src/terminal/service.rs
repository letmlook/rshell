//! 终端服务
//!
//! 集成 alacritty_terminal 进行 VT 序列解析和终端状态管理。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions as GridDimensions;
use alacritty_terminal::term::cell::{Cell, Flags as AlacrittyFlags};
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::{Processor, NamedColor};
use rshell_api::types::{CellFlags, CellView, TerminalBufferSnapshot, TerminalConfig};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, instrument};
use uuid::Uuid;

/// 终端尺寸（实现 alacritty_terminal 的 Dimensions trait）
struct TermSize {
    columns: usize,
    screen_lines: usize,
}

impl TermSize {
    fn new(columns: usize, screen_lines: usize) -> Self {
        Self { columns, screen_lines }
    }
}

impl GridDimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// 终端实例
struct TerminalInstance {
    /// 终端 ID
    #[allow(dead_code)]
    id: Uuid,
    /// 终端配置
    #[allow(dead_code)]
    config: TerminalConfig,
    /// alacritty_terminal 终端实例
    term: Term<VoidListener>,
    /// VT 序列解析器
    parser: Processor,
    /// 终端标题（我们自己跟踪，因为 Term 的 title 字段是私有的）
    title: String,
}

impl TerminalInstance {
    /// 创建新的终端实例
    fn new(id: Uuid, config: TerminalConfig, cols: usize, rows: usize) -> Self {
        let size = TermSize::new(cols, rows);
        let term_config = TermConfig::default();
        let term = Term::new(term_config, &size, VoidListener);
        let parser = Processor::new();

        Self {
            id,
            config,
            term,
            parser,
            title: String::new(),
        }
    }

    /// 处理输入数据（VT 序列解析）
    fn process_bytes(&mut self, data: &[u8]) {
        self.parser.advance(&mut self.term, data);
    }

    /// 调整终端大小
    fn resize(&mut self, cols: usize, rows: usize) {
        let size = TermSize::new(cols, rows);
        self.term.resize(size);
    }

    /// 获取缓冲区快照
    fn get_snapshot(&self) -> TerminalBufferSnapshot {
        let grid = self.term.grid();
        let rows = grid.screen_lines();
        let cols = grid.columns();

        let mut cells = Vec::with_capacity(rows * cols);

        // 遍历网格中的每个单元格
        for line_idx in 0..rows {
            let line = alacritty_terminal::index::Line(line_idx as i32);
            for col_idx in 0..cols {
                let column = alacritty_terminal::index::Column(col_idx);
                let cell = &grid[line][column];

                cells.push(cell_to_view(cell));
            }
        }

        // 获取光标位置
        let cursor = self.term.grid().cursor.point;
        let cursor_row = cursor.line.0.max(0) as usize;
        let cursor_col = cursor.column.0;

        TerminalBufferSnapshot {
            rows,
            cols,
            cells,
            cursor_row,
            cursor_col,
            cursor_visible: self.term.mode().contains(alacritty_terminal::term::TermMode::SHOW_CURSOR),
            title: self.title.clone(),
        }
    }
}

/// 将 alacritty_terminal 的 Cell 转换为 API 的 CellView
fn cell_to_view(cell: &Cell) -> CellView {
    let character = cell.c;
    let flags = cell.flags;

    // 转换标志位
    let mut cell_flags = CellFlags::empty();
    if flags.contains(AlacrittyFlags::INVERSE) {
        cell_flags.insert(CellFlags::REVERSE);
    }
    if flags.contains(AlacrittyFlags::BOLD) {
        cell_flags.insert(CellFlags::BOLD);
    }
    if flags.contains(AlacrittyFlags::ITALIC) {
        cell_flags.insert(CellFlags::ITALIC);
    }
    if flags.contains(AlacrittyFlags::UNDERLINE) {
        cell_flags.insert(CellFlags::UNDERLINE);
    }
    if flags.contains(AlacrittyFlags::STRIKEOUT) {
        cell_flags.insert(CellFlags::STRIKETHROUGH);
    }
    if flags.contains(AlacrittyFlags::DIM) {
        cell_flags.insert(CellFlags::DIM);
    }
    if flags.contains(AlacrittyFlags::HIDDEN) {
        cell_flags.insert(CellFlags::HIDDEN);
    }

    // 提取实际颜色
    let fg_color = color_to_rgba(&cell.fg, true);
    let bg_color = color_to_rgba(&cell.bg, false);

    CellView {
        character,
        fg_color,
        bg_color,
        flags: cell_flags,
    }
}

/// 将 alacritty_terminal 的 Color 转换为 RGBA
fn color_to_rgba(color: &alacritty_terminal::vte::ansi::Color, is_fg: bool) -> [u8; 4] {
    use alacritty_terminal::vte::ansi::Color::*;
    match color {
        Named(named) => {
            match named {
                NamedColor::Black => [0, 0, 0, 255],
                NamedColor::Red => [205, 0, 0, 255],
                NamedColor::Green => [0, 205, 0, 255],
                NamedColor::Yellow => [205, 205, 0, 255],
                NamedColor::Blue => [0, 0, 238, 255],
                NamedColor::Magenta => [205, 0, 205, 255],
                NamedColor::Cyan => [0, 205, 205, 255],
                NamedColor::White => [229, 229, 229, 255],
                NamedColor::BrightBlack => [127, 127, 127, 255],
                NamedColor::BrightRed => [255, 0, 0, 255],
                NamedColor::BrightGreen => [0, 255, 0, 255],
                NamedColor::BrightYellow => [255, 255, 0, 255],
                NamedColor::BrightBlue => [92, 92, 255, 255],
                NamedColor::BrightMagenta => [255, 0, 255, 255],
                NamedColor::BrightCyan => [0, 255, 255, 255],
                NamedColor::BrightWhite => [255, 255, 255, 255],
                NamedColor::Foreground => if is_fg { [205, 214, 244, 255] } else { [0, 0, 0, 0] },
                NamedColor::Background => if is_fg { [0, 0, 0, 0] } else { [30, 30, 46, 255] },
                _ => [128, 128, 128, 255],
            }
        }
        Spec(rgb) => [rgb.r, rgb.g, rgb.b, 255],
        Indexed(idx) => {
            // 256 色表（简化：前 16 色为标准 ANSI，其余为灰度）
            let idx = *idx;
            if idx < 16 {
                let colors: [[u8; 4]; 16] = [
                    [0, 0, 0, 255], [205, 0, 0, 255], [0, 205, 0, 255], [205, 205, 0, 255],
                    [0, 0, 238, 255], [205, 0, 205, 255], [0, 205, 205, 255], [229, 229, 229, 255],
                    [127, 127, 127, 255], [255, 0, 0, 255], [0, 255, 0, 255], [255, 255, 0, 255],
                    [92, 92, 255, 255], [255, 0, 255, 255], [0, 255, 255, 255], [255, 255, 255, 255],
                ];
                colors[idx as usize]
            } else if idx < 232 {
                // 216 色立方体
                let idx = idx - 16;
                let r = (idx / 36) % 6;
                let g = (idx / 6) % 6;
                let b = idx % 6;
                let to_val = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                [to_val(r), to_val(g), to_val(b), 255]
            } else {
                // 灰度
                let v = 8 + (idx - 232) * 10;
                [v, v, v, 255]
            }
        }
    }
}

/// 终端服务 - 管理终端实例的生命周期
pub struct TerminalService {
    /// 终端实例映射
    terminals: Arc<RwLock<HashMap<Uuid, TerminalInstance>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
}

impl TerminalService {
    /// 创建新的终端服务
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            terminals: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    /// 创建终端实例
    #[instrument(skip(self, config))]
    pub fn create_terminal(&self, id: Uuid, config: TerminalConfig) -> Result<(), CoreError> {
        info!(terminal_id = %id, "Creating terminal");

        let instance = TerminalInstance::new(id, config, 80, 24);

        let mut terminals = self.terminals.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        terminals.insert(id, instance);

        debug!(terminal_id = %id, "Terminal created");
        Ok(())
    }

    /// 销毁终端实例
    pub fn destroy_terminal(&self, id: Uuid) -> Result<(), CoreError> {
        info!(terminal_id = %id, "Destroying terminal");

        let mut terminals = self.terminals.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        terminals.remove(&id);

        debug!(terminal_id = %id, "Terminal destroyed");
        Ok(())
    }

    /// 处理来自远程主机的输出数据（VT 序列解析）
    #[instrument(skip(self, data))]
    pub fn process_output(&self, terminal_id: Uuid, data: &[u8]) -> Result<(), CoreError> {
        debug!(terminal_id = %terminal_id, bytes = data.len(), "Processing terminal output");

        let mut terminals = self.terminals.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        let instance = terminals
            .get_mut(&terminal_id)
            .ok_or_else(|| CoreError::NotFound(format!("Terminal {} not found", terminal_id)))?;

        // 使用 alacritty_terminal 解析 VT 序列
        instance.process_bytes(data);

        Ok(())
    }

    /// 发送输入到终端（用户输入，发送到远程主机）
    ///
    /// **已弃用**:用户输入应直接通过 `SessionService::send_data(session_id, data)`
    /// 发往远端,不应再回流到 `TerminalService` 重新发布 `TerminalOutput` 事件 —
    /// 那会构成"输入 → 假输出 → 再次解析"的回路,而且前端 View 已自行渲染按键回显。
    ///
    /// 保留该方法仅为 API 兼容性;调用方会收到 deprecation warning。
    #[deprecated(
        since = "0.1.0",
        note = "Use SessionService::send_data directly; this method echoes input as TerminalOutput which forms a feedback loop"
    )]
    #[instrument(skip(self, data))]
    pub fn send_input(&self, terminal_id: Uuid, data: &[u8]) -> Result<(), CoreError> {
        debug!(terminal_id = %terminal_id, bytes = data.len(), "Sending terminal input (deprecated path)");

        // 保留旧的实现以避免 hard break，但不再被 CommandDispatcher 调用
        self.event_bus.publish(rshell_api::AppEvent::TerminalOutput {
            session_id: terminal_id,
            data: data.to_vec(),
        });

        Ok(())
    }

    /// 调整终端大小
    #[instrument(skip(self))]
    pub fn resize(&self, terminal_id: Uuid, cols: u16, rows: u16) -> Result<(), CoreError> {
        debug!(terminal_id = %terminal_id, cols, rows, "Resizing terminal");

        let mut terminals = self.terminals.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        let instance = terminals
            .get_mut(&terminal_id)
            .ok_or_else(|| CoreError::NotFound(format!("Terminal {} not found", terminal_id)))?;

        // 使用 alacritty_terminal 调整缓冲区大小
        instance.resize(cols as usize, rows as usize);

        Ok(())
    }

    /// 获取终端缓冲区快照
    pub fn get_buffer_snapshot(&self, terminal_id: Uuid) -> Result<TerminalBufferSnapshot, CoreError> {
        let terminals = self.terminals.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        let instance = terminals
            .get(&terminal_id)
            .ok_or_else(|| CoreError::NotFound(format!("Terminal {} not found", terminal_id)))?;

        Ok(instance.get_snapshot())
    }
}
