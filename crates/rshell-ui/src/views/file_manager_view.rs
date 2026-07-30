//! 文件管理器视图
//!
//! 双窗格文件管理器：左侧本地文件，右侧远程文件。
//! 支持文件上传/下载操作。

use gpui::*;
use rshell_api::types::RemoteFileEntry;
use std::path::PathBuf;

/// 文件管理器视图
pub struct FileManagerView {
    /// 当前会话 ID
    session_id: Option<uuid::Uuid>,
    /// 本地当前路径
    local_path: PathBuf,
    /// 远程当前路径
    remote_path: String,
    /// 本地文件列表
    local_files: Vec<LocalFileEntry>,
    /// 远程文件列表
    remote_files: Vec<RemoteFileEntry>,
    /// 选中的本地文件索引
    selected_local: Option<usize>,
    /// 选中的远程文件索引
    selected_remote: Option<usize>,
    /// 活动面板（0=本地，1=远程）
    active_panel: u8,
}

/// 本地文件条目
#[derive(Debug, Clone)]
struct LocalFileEntry {
    name: String,
    is_dir: bool,
    size: u64,
    modified: String,
}

impl FileManagerView {
    /// 创建新的文件管理器视图
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            session_id: None,
            local_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            remote_path: "/".to_string(),
            local_files: Vec::new(),
            remote_files: Vec::new(),
            selected_local: None,
            selected_remote: None,
            active_panel: 0,
        }
    }

    /// 设置会话 ID
    pub fn set_session_id(&mut self, session_id: uuid::Uuid) {
        self.session_id = Some(session_id);
    }

    /// 更新远程文件列表
    pub fn update_remote_files(&mut self, path: &str, files: Vec<RemoteFileEntry>) {
        self.remote_path = path.to_string();
        self.remote_files = files;
        self.selected_remote = None;
    }

    /// 刷新本地文件列表
    pub fn refresh_local_files(&mut self) {
        self.local_files.clear();
        
        if let Ok(entries) = std::fs::read_dir(&self.local_path) {
            for entry in entries.flatten() {
                let metadata = entry.metadata().ok();
                let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified = metadata
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_default();

                self.local_files.push(LocalFileEntry {
                    name: entry.file_name().to_string_lossy().to_string(),
                    is_dir,
                    size,
                    modified,
                });
            }
        }

        // 排序：目录在前，然后按名称排序
        self.local_files.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        self.selected_local = None;
    }

    /// 导航到本地路径
    pub fn navigate_local(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.local_path = path;
            self.refresh_local_files();
        }
    }

    /// 导航到远程路径
    pub fn navigate_remote(&mut self, path: String) {
        self.remote_path = path;
        // 实际实现需要通过 CommandDispatcher 请求远程目录列表
    }

    /// 向上一级（本地）
    pub fn go_up_local(&mut self) {
        if let Some(parent) = self.local_path.parent() {
            self.navigate_local(parent.to_path_buf());
        }
    }

    /// 进入目录（本地）
    pub fn enter_dir_local(&mut self, index: usize) {
        if index < self.local_files.len() && self.local_files[index].is_dir {
            let new_path = self.local_path.join(&self.local_files[index].name);
            self.navigate_local(new_path);
        }
    }

    /// 获取选中的本地文件路径
    pub fn get_selected_local_path(&self) -> Option<PathBuf> {
        self.selected_local.map(|i| {
            if i < self.local_files.len() {
                self.local_path.join(&self.local_files[i].name)
            } else {
                self.local_path.clone()
            }
        })
    }

    /// 获取选中的远程文件路径
    pub fn get_selected_remote_path(&self) -> Option<String> {
        self.selected_remote.map(|i| {
            if i < self.remote_files.len() {
                format!("{}/{}", self.remote_path.trim_end_matches('/'), self.remote_files[i].name)
            } else {
                self.remote_path.clone()
            }
        })
    }
}

impl Render for FileManagerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let local_path_str = self.local_path.to_string_lossy().to_string();
        let remote_path_str = self.remote_path.clone();
        let session_id_local = self.session_id;
        let session_id_remote = self.session_id;
        // 闭包所需: clone selected_local/remote + files 列表
        // removed local_files_clone: 直接 iter self.local_files
        // removed remote_files_clone
        // removed local_path_clone
        // removed remote_path_clone

        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(40.0))
                    .bg(rgb(0x181825))
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .gap(px(8.0))
                    .child(
                        div()
                            .id(("file-upload-btn", 0usize))
                            .bg(rgb(0x3b82f6))
                            .px(px(12.0))
                            .py(px(6.0))
                            .rounded(px(4.0))
                            .text_color(rgb(0xffffff))
                            .text_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x2563eb)))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                let Some(sid) = session_id_local else { return };
                                let Some(local) = this.get_selected_local_path() else { return };
                                let remote = this.get_selected_remote_path().unwrap_or_default();
                                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                    bridge.send_command(rshell_api::AppCommand::EnqueueUpload {
                                        local,
                                        remote,
                                        session_id: sid,
                                    });
                                }
                            }))
                            .child("↑ 上传"),
                    )
                    .child(
                        div()
                            .id(("file-download-btn", 0usize))
                            .bg(rgb(0x10b981))
                            .px(px(12.0))
                            .py(px(6.0))
                            .rounded(px(4.0))
                            .text_color(rgb(0xffffff))
                            .text_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x059669)))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                let Some(sid) = session_id_remote else { return };
                                let Some(remote) = this.get_selected_remote_path() else { return };
                                let local = this
                                    .get_selected_local_path()
                                    .unwrap_or_else(|| this.local_path.clone());
                                if let Some(bridge) = cx.try_global::<crate::bridge::AppBridge>() {
                                    bridge.send_command(rshell_api::AppCommand::EnqueueDownload {
                                        remote,
                                        local,
                                        session_id: sid,
                                    });
                                }
                            }))
                            .child("↓ 下载"),
                    ),
            )
            // 双窗格区域
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .gap(px(2.0))
                    .p(px(8.0))
                    // 本地文件面板
                    .child(
                        div()
                            .flex_1()
                            .bg(rgb(0x181825))
                            .rounded(px(6.0))
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            // 本地路径栏
                            .child(
                                div()
                                    .h(px(32.0))
                                    .bg(rgb(0x1e1e2e))
                                    .flex()
                                    .items_center()
                                    .px(px(8.0))
                                    .child(
                                        div()
                                            .text_color(rgb(0xcdd6f4))
                                            .text_sm()
                                            .child(format!("本地: {}", local_path_str)),
                                    ),
                            )
                            // 本地文件列表
                            .child(
                                div()
                                    .flex_1()
                                    .children(self.local_files.iter().enumerate().map(|(i, file)| {
                                        let is_selected = self.selected_local == Some(i);
                                        let bg_color = if is_selected { rgb(0x3b82f6) } else { rgb(0x181825) };
                                        let icon = if file.is_dir { "📁" } else { "📄" };

                                        div()
                                            .h(px(28.0))
                                            .bg(bg_color)
                                            .flex()
                                            .items_center()
                                            .px(px(8.0))
                                            .gap(px(8.0))
                                            .child(div().text_color(rgb(0xcdd6f4)).child(icon))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_color(rgb(0xcdd6f4))
                                                    .text_sm()
                                                    .child(file.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_color(rgb(0x6c7086))
                                                    .text_xs()
                                                    .child(format_size(file.size)),
                                            )
                                    })),
                            ),
                    )
                    // 远程文件面板
                    .child(
                        div()
                            .flex_1()
                            .bg(rgb(0x181825))
                            .rounded(px(6.0))
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            // 远程路径栏
                            .child(
                                div()
                                    .h(px(32.0))
                                    .bg(rgb(0x1e1e2e))
                                    .flex()
                                    .items_center()
                                    .px(px(8.0))
                                    .child(
                                        div()
                                            .text_color(rgb(0xcdd6f4))
                                            .text_sm()
                                            .child(format!("远程: {}", remote_path_str)),
                                    ),
                            )
                            // 远程文件列表
                            .child(
                                div()
                                    .flex_1()
                                    .children(self.remote_files.iter().enumerate().map(|(i, file)| {
                                        let is_selected = self.selected_remote == Some(i);
                                        let bg_color = if is_selected { rgb(0x3b82f6) } else { rgb(0x181825) };
                                        let icon = match file.file_type {
                                            rshell_api::types::FileType::Directory => "📁",
                                            rshell_api::types::FileType::Symlink => "🔗",
                                            _ => "📄",
                                        };

                                        div()
                                            .h(px(28.0))
                                            .bg(bg_color)
                                            .flex()
                                            .items_center()
                                            .px(px(8.0))
                                            .gap(px(8.0))
                                            .child(div().text_color(rgb(0xcdd6f4)).child(icon))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_color(rgb(0xcdd6f4))
                                                    .text_sm()
                                                    .child(file.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_color(rgb(0x6c7086))
                                                    .text_xs()
                                                    .child(format_size(file.size)),
                                            )
                                    })),
                            ),
                    ),
            )
    }
}

/// 格式化文件大小
fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "-".to_string();
    }
    
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
