//! 传输 ViewModel

use rshell_api::types::TransferDirection as ApiTransferDirection;
use uuid::Uuid;

/// 传输任务状态
#[derive(Debug, Clone)]
pub struct TransferTaskView {
    pub task_id: Uuid,
    pub filename: String,
    pub direction: TransferDirection,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub state: String,
}

/// 传输方向
#[derive(Debug, Clone, Copy)]
pub enum TransferDirection {
    Upload,
    Download,
}

/// 传输 ViewModel
pub struct TransferViewModel {
    /// 传输任务列表
    pub tasks: Vec<TransferTaskView>,
    /// 当前选中的任务
    pub selected_task: Option<usize>,
}

impl TransferViewModel {
    /// 创建新的 ViewModel
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            selected_task: None,
        }
    }

    /// 处理事件
    pub fn handle_event(&mut self, event: &rshell_api::AppEvent) {
        match event {
            rshell_api::AppEvent::TransferProgress { task_id, bytes, total, speed_bps } => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.task_id == *task_id) {
                    task.bytes_transferred = *bytes;
                    task.total_bytes = *total;
                    task.speed_bps = *speed_bps;
                }
            }
            rshell_api::AppEvent::TransferTaskAdded { task_id, filename, direction } => {
                let dir = match direction {
                    ApiTransferDirection::Upload => TransferDirection::Upload,
                    ApiTransferDirection::Download => TransferDirection::Download,
                };
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
            rshell_api::AppEvent::TransferTaskCompleted { task_id } => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.task_id == *task_id) {
                    task.state = "已完成".to_string();
                }
            }
            rshell_api::AppEvent::TransferTaskFailed { task_id, error } => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.task_id == *task_id) {
                    task.state = format!("失败: {}", error);
                }
            }
            _ => {}
        }
    }

    /// 获取进度百分比
    pub fn progress_percent(&self, task: &TransferTaskView) -> f64 {
        if task.total_bytes == 0 {
            0.0
        } else {
            (task.bytes_transferred as f64 / task.total_bytes as f64) * 100.0
        }
    }

    /// 格式化速度
    pub fn format_speed(speed_bps: f64) -> String {
        if speed_bps < 1024.0 {
            format!("{:.0} B/s", speed_bps)
        } else if speed_bps < 1024.0 * 1024.0 {
            format!("{:.1} KB/s", speed_bps / 1024.0)
        } else {
            format!("{:.1} MB/s", speed_bps / 1024.0 / 1024.0)
        }
    }

    /// 移除已完成的任务
    pub fn clear_completed(&mut self) {
        self.tasks.retain(|t| t.state != "已完成");
    }
}
