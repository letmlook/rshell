//! 共享存储工具
//!
//! 各功能模块（快速命令、触发器、连接分组…）都要把 JSON 原子地落盘。
//! 这份实现原先住在 `script::quick_command` 里，被别的模块借用——
//! 共享工具藏在某个功能模块下，依赖方向是反的。放在这里，各模块按需取用。

use std::path::Path;

use crate::error::CoreError;

/// 把 JSON 原子地写入 `path`
///
/// 同目录写暂存文件再 rename：读者要么看到旧文件、要么看到新文件，不会读到
/// 半份内容（写到一半崩溃留下的截断文件会让下次启动直接读不出来）。
///
/// 暂存名固定为 `<path>.tmp`：不与真实文件同 inode，rename 才能原子替换。
pub(crate) fn persist_json(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CoreError::StorageError(e.to_string()))?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| CoreError::StorageError(e.to_string()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        // 替换失败时清掉暂存文件，别把半份内容留在数据目录里
        let _ = std::fs::remove_file(&tmp);
        return Err(CoreError::StorageError(e.to_string()));
    }
    Ok(())
}
