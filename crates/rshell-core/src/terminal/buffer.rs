//! 终端缓冲区管理
//!
//! 缓冲区由 alacritty_terminal 的 Term 管理，此模块提供辅助功能。

use rshell_api::types::TerminalBufferSnapshot;

/// 从快照创建空的缓冲区快照（用于初始化）
pub fn empty_snapshot(rows: usize, cols: usize) -> TerminalBufferSnapshot {
    let cells = vec![Default::default(); rows * cols];
    TerminalBufferSnapshot {
        rows,
        cols,
        cells,
        cursor_row: 0,
        cursor_col: 0,
        cursor_visible: true,
        title: String::new(),
    }
}
